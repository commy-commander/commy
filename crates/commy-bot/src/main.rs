use anyhow::{bail, Context, Result};
use commy_bot::moderator::MAX_BAN_AFTER;
use commy_bot::runner::{require_profile, run, status_line};
use commy_bot::storage::{ticket_in, DataDir};
use commy_bot::Policy;

const USAGE: &str = "\
commy-bot: a Commy member that runs on its own and keeps chat tidy

Usage:
  commy-bot [--data FOLDER] init NAME
  commy-bot [--data FOLDER] join INVITE
  commy-bot [--data FOLDER] servers
  commy-bot [--data FOLDER] run [--log-channel NAME] [--ban-after COUNT]

FOLDER defaults to $COMMY_BOT_DATA, then ./commy-bot-data.
INVITE is the code from Invite People > Copy Invite Code in the app (it starts with commy).
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

fn parse_policy(mut flags: &[String]) -> Result<Policy> {
    let mut policy = Policy::default();
    while let [flag, value, rest @ ..] = flags {
        match flag.as_str() {
            "--log-channel" => policy.log_channel = Some(value.trim_start_matches('#').to_string()),
            "--ban-after" => {
                let count: u32 = value.parse().ok().filter(|n| (1..=MAX_BAN_AFTER).contains(n)).with_context(|| {
                    format!("--ban-after takes a count from 1 to {MAX_BAN_AFTER}")
                })?;
                policy.ban_after = Some(count);
            }
            other => bail!("unknown option {other}\n\n{USAGE}"),
        }
        flags = rest;
    }
    if let [flag] = flags {
        bail!("{flag} needs a value\n\n{USAGE}");
    }
    Ok(policy)
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
            let policy = parse_policy(&flags)?;
            let stop = async {
                let _ = tokio::signal::ctrl_c().await;
            };
            run(&args.data, policy, None, stop).await?;
        }
        _ => print!("{USAGE}"),
    }
    Ok(())
}
