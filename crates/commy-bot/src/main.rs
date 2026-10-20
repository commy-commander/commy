use anyhow::{bail, Context, Result};
use commy_bot::moderator::MAX_BAN_AFTER;
use commy_bot::runner::{require_profile, run, status_line, Setup};
use commy_bot::storage::{ticket_in, DataDir};
use commy_bot::webhook::{self, Webhook};
use commy_bot::{bridge, compat};
use std::net::SocketAddr;

const USAGE: &str = "\
commy-bot: a Commy member that runs on its own and keeps chat tidy

Usage:
  commy-bot [--data FOLDER] init NAME
  commy-bot [--data FOLDER] join INVITE
  commy-bot [--data FOLDER] servers
  commy-bot [--data FOLDER] run [--log-channel NAME] [--ban-after COUNT] [--bridge [PORT]] [--webhooks [ADDRESS]]
  commy-bot [--data FOLDER] token [--reset]
  commy-bot [--data FOLDER] webhook add SERVER CHANNEL [NAME]
  commy-bot [--data FOLDER] webhook list
  commy-bot [--data FOLDER] webhook remove ID

FOLDER defaults to $COMMY_BOT_DATA, then ./commy-bot-data.
INVITE is the code from Invite People > Copy Invite Code in the app (it starts with commy).
--bridge lets bot code in any language connect on 127.0.0.1 (port 47620 unless given) with the token.
--webhooks accepts webhook posts on 127.0.0.1:47621 unless an address is given.
";

struct Args {
    data: DataDir,
    command: Vec<String>,
}

fn parse_args(mut raw: Vec<String>) -> Result<Args> {
    let mut data = std::env::var("COMMY_BOT_DATA").unwrap_or_else(|_| "commy-bot-data".into());
    if raw.first().map(String::as_str) == Some("--data") {
        raw.remove(0);
        if raw.is_empty() {
            bail!("--data needs a folder");
        }
        data = raw.remove(0);
    }
    Ok(Args { data: DataDir::new(data), command: raw })
}

fn parse_setup(mut flags: &[String]) -> Result<Setup> {
    let mut setup = Setup::default();
    while let [flag, rest @ ..] = flags {
        let (value, after) = match rest {
            [value, after @ ..] if !value.starts_with("--") => (Some(value.as_str()), after),
            _ => (None, rest),
        };
        let needed = || value.with_context(|| format!("{flag} needs a value\n\n{USAGE}"));
        match flag.as_str() {
            "--log-channel" => setup.policy.log_channel = Some(needed()?.trim_start_matches('#').to_string()),
            "--ban-after" => {
                let count: u32 = needed()?.parse().ok().filter(|n| (1..=MAX_BAN_AFTER).contains(n)).with_context(|| {
                    format!("--ban-after takes a count from 1 to {MAX_BAN_AFTER}")
                })?;
                setup.policy.ban_after = Some(count);
            }
            "--bridge" => {
                let port = value.map(str::parse::<u16>).transpose().ok().flatten().filter(|p| *p != 0);
                if value.is_some() && port.is_none() {
                    bail!("--bridge takes a port from 1 to 65535");
                }
                setup.bridge = Some(port.unwrap_or(bridge::DEFAULT_PORT));
            }
            "--webhooks" => {
                let addr = value.map(webhook_addr).transpose()?;
                setup.webhooks = Some(addr.unwrap_or_else(webhook::default_addr));
            }
            other => bail!("unknown option {other}\n\n{USAGE}"),
        }
        flags = after;
    }
    Ok(setup)
}

fn webhook_addr(text: &str) -> Result<SocketAddr> {
    if let Ok(port) = text.parse::<u16>() {
        return Ok(SocketAddr::from(([127, 0, 0, 1], port)));
    }
    text.parse().with_context(|| format!("--webhooks takes a port or an address like 127.0.0.1:{}", webhook::DEFAULT_PORT))
}

fn add_webhook(data: &DataDir, server: &str, channel: &str, name: &str) -> Result<Webhook> {
    let servers = data.servers()?;
    let server = server.trim();
    let (ticket, state) = servers
        .iter()
        .find(|(t, _)| t.server_id.to_string() == server)
        .or_else(|| servers.iter().find(|(t, s)| s.as_ref().map_or(&t.server_name, |s| &s.server.name).eq_ignore_ascii_case(server)))
        .context("the bot is not in that server; see commy-bot servers")?;
    let state = state.as_ref().context("start the bot once so it has a copy of the server, then add the webhook")?;
    let channel = compat::channel(state, channel).context("no text channel by that name")?;
    let name = if name.trim().is_empty() { "Webhook" } else { name };
    data.add_webhook(Webhook::new(name, ticket.server_id, channel))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "commy_bot=info,warn".into()),
        )
        .init();
    let args = parse_args(std::env::args().skip(1).collect())?;
    match args.command.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["init", name @ ..] if !name.is_empty() => {
            let profile = args.data.create_profile(&name.join(" "))?;
            println!("Created {} in {}", profile.username, args.data.root().display());
            println!("Next: commy-bot join INVITE, then give it a role with Manage Messages.");
        }
        ["join", text @ ..] if !text.is_empty() => {
            require_profile(&args.data)?;
            let ticket = ticket_in(&text.join(" "))?;
            if args.data.has_server(ticket.server_id) {
                println!("Already added {}.", ticket.server_name);
            } else {
                args.data.save_server(&ticket, None)?;
                println!("Added {}. Start the bot with: commy-bot run", ticket.server_name);
            }
        }
        ["servers"] => {
            require_profile(&args.data)?;
            let servers = args.data.servers()?;
            if servers.is_empty() {
                println!("No servers yet. Add one with: commy-bot join INVITE");
            }
            for (ticket, state) in servers {
                let line = match &state {
                    None => "waiting for the owner to come online".to_string(),
                    Some(state) => status_line(state, args.data.profile()?.map(|p| p.id)),
                };
                println!("{}: {line}", state.as_ref().map_or(&ticket.server_name, |s| &s.server.name));
            }
        }
        ["run", flags @ ..] => {
            let flags: Vec<String> = flags.iter().map(|s| s.to_string()).collect();
            let setup = parse_setup(&flags)?;
            let stop = async {
                let _ = tokio::signal::ctrl_c().await;
            };
            run(&args.data, setup, None, stop).await?;
        }
        ["token", rest @ ..] if rest.is_empty() || rest == ["--reset"] => {
            require_profile(&args.data)?;
            println!("{}", args.data.bridge_token(!rest.is_empty())?);
        }
        ["webhook", "add", server, channel, name @ ..] => {
            require_profile(&args.data)?;
            let hook = add_webhook(&args.data, server, channel, &name.join(" "))?;
            let saved = args.data.webhook_url_file(&hook.id);
            println!("Added {} ({}).", hook.name, hook.id);
            println!("Its secret URL is in {}. POST JSON like {{\"content\": \"hello\"}} to it.", saved.display());
            println!("Keep the URL private; start the bot with: commy-bot run --webhooks");
        }
        ["webhook", "list"] => {
            let hooks = args.data.webhooks()?;
            if hooks.is_empty() {
                println!("No webhooks yet. Add one with: commy-bot webhook add SERVER CHANNEL [NAME]");
            }
            for hook in hooks {
                println!("{}  {}  URL in {}", hook.id, hook.name, args.data.webhook_url_file(&hook.id).display());
            }
        }
        ["webhook", "remove", id] => match args.data.remove_webhook(id)? {
            true => println!("Removed {id}; its URL stops working right away."),
            false => bail!("no webhook with id {id}"),
        },
        _ => print!("{USAGE}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(flags: &[&str]) -> Result<Setup> {
        parse_setup(&flags.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn run_options_take_optional_values() {
        let plain = setup(&["--bridge", "--webhooks"]).unwrap();
        assert_eq!(plain.bridge, Some(bridge::DEFAULT_PORT));
        assert_eq!(plain.webhooks, Some(webhook::default_addr()));
        let chosen = setup(&["--log-channel", "#mod-log", "--bridge", "5000", "--webhooks", "0.0.0.0:8080", "--ban-after", "3"]).unwrap();
        assert_eq!(chosen.policy.log_channel.as_deref(), Some("mod-log"));
        assert_eq!(chosen.policy.ban_after, Some(3));
        assert_eq!(chosen.bridge, Some(5000));
        assert_eq!(chosen.webhooks, Some("0.0.0.0:8080".parse().unwrap()));
        assert_eq!(setup(&["--webhooks", "9000"]).unwrap().webhooks, Some("127.0.0.1:9000".parse().unwrap()));
        assert!(setup(&["--bridge", "lots"]).is_err());
        assert!(setup(&["--log-channel"]).is_err());
        assert!(setup(&["--ban-after", "0"]).is_err());
        assert!(setup(&["--fly"]).is_err());
        assert_eq!(setup(&[]).unwrap(), Setup::default());
    }
}
