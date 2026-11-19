use std::time::Duration;

use commy_core::{Identity, ServerEvent, SignedEvent, UserProfile};
use commy_net::community_streams as streams;
use commy_net::invite::{self, ResolveResponse};
use commy_net::WirePayload;
use commy_sync::SyncClient;

const LINKS: usize = 4;
const GAP: Duration = Duration::from_millis(125);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [link] = args.as_slice() {
        let parsed = invite::parse(link).ok_or_else(|| anyhow::anyhow!("not a Commy invite link"))?;
        println!("{}", parsed.endpoint(invite::RESOLVE_PATH));
        return Ok(());
    }
    let [link, resolved, sync_url, count] = args.as_slice() else {
        anyhow::bail!("usage: seed_members <invite link>  (prints its resolve url)\n       seed_members <invite link> <resolve reply json file> <sync wss url, or auto> <member count>");
    };
    let parsed = invite::parse(link).ok_or_else(|| anyhow::anyhow!("not a Commy invite link"))?;
    let reply: ResolveResponse = serde_json::from_slice(&std::fs::read(resolved)?)?;
    let opened = invite::open_reply(&parsed, &reply, chrono::Utc::now()).map_err(anyhow::Error::msg)?;
    let (key, secret) = opened.community.ok_or_else(|| anyhow::anyhow!("this invite carries no community key"))?;
    let server = opened.ticket.server_id;
    let keys = streams::bootstrap(server, key, &secret).keys;
    let count: usize = count.parse()?;
    let sync_url = match sync_url.as_str() {
        "auto" => commy_core::sync_host::socket_url(opened.sync_host.as_deref().unwrap_or(&parsed.origin))
            .ok_or_else(|| anyhow::anyhow!("no sync host for this invite"))?,
        given => given.to_string(),
    };
    println!("seeding {count} members into {} through {sync_url}", opened.ticket.server_name);
    let links: Vec<SyncClient> = (0..LINKS)
        .map(|_| {
            let (client, _) = SyncClient::spawn(sync_url.clone());
            client.follow_tail(keys.clone(), 0, Some(0));
            client
        })
        .collect();
    let mut writes = Vec::with_capacity(count);
    for i in 0..count {
        let identity = Identity::generate();
        let mut profile = UserProfile::new(format!("seed-{i:05}"));
        profile.identity = Some(identity.public());
        let at = chrono::Utc::now().timestamp_millis();
        let join = SignedEvent::sign(server, profile.id, at, ServerEvent::MemberJoin { server, profile: profile.clone() }, &identity);
        let plain = streams::encode_entry(&WirePayload::Signed(join)).ok_or_else(|| anyhow::anyhow!("a join did not encode"))?;
        let client = links[i % LINKS].clone();
        let stream = keys.id;
        writes.push(tokio::spawn(async move { client.append(stream, plain).await }));
        tokio::time::sleep(GAP).await;
        if (i + 1) % 250 == 0 {
            println!("sent {} joins", i + 1);
        }
    }
    let mut stored = 0;
    for write in writes {
        match write.await? {
            Ok(_) => stored += 1,
            Err(e) => eprintln!("a join was refused: {e}"),
        }
    }
    println!("stored {stored} of {count} joins");
    Ok(())
}
