use lettre::message::{header::ContentType, Mailbox};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::config::SmtpSettings;
use crate::reports::{describe, StoredReport};

const IMPLICIT_TLS_PORT: u16 = 465;
const SUBJECT_NAME_CHARS: usize = 60;

#[derive(Clone)]
pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    to: Mailbox,
}

impl Mailer {
    pub fn new(settings: &SmtpSettings, to: &str) -> Result<Self, String> {
        let transport = match settings {
            SmtpSettings::Url { url, .. } => AsyncSmtpTransport::<Tokio1Executor>::from_url(url).map_err(|e| e.to_string())?.build(),
            SmtpSettings::Host { host, port, user, password, .. } => {
                let builder = if *port == IMPLICIT_TLS_PORT {
                    AsyncSmtpTransport::<Tokio1Executor>::relay(host)
                } else {
                    AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
                }
                .map_err(|e| e.to_string())?
                .port(*port);
                match (user, password) {
                    (Some(user), Some(password)) => builder.credentials(Credentials::new(user.clone(), password.clone())).build(),
                    _ => builder.build(),
                }
            }
        };
        let from = settings.from().parse::<Mailbox>().map_err(|e| format!("EMAIL_FROM: {e}"))?;
        let to = to.parse::<Mailbox>().map_err(|e| format!("SYNC_ABUSE_CONTACT: {e}"))?;
        Ok(Self { transport, from, to })
    }

    pub async fn send(&self, stored: &StoredReport) -> Result<(), String> {
        self.send_text(&subject(stored), &body(stored)).await
    }

    pub async fn send_text(&self, subject: &str, text: &str) -> Result<(), String> {
        let message = Message::builder()
            .from(self.from.clone())
            .to(self.to.clone())
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(text.to_string())
            .map_err(|e| e.to_string())?;
        self.transport.send(message).await.map(|_| ()).map_err(|e| e.to_string())
    }
}

pub fn subject(stored: &StoredReport) -> String {
    let reason: String = stored.report.reason().chars().filter(|c| !c.is_control()).collect();
    if stored.report.is_sealed() {
        return format!("Commy report {}: {} (sealed)", stored.id, reason);
    }
    let name: String = stored.report.community_name().chars().filter(|c| !c.is_control()).take(SUBJECT_NAME_CHARS).collect();
    format!("Commy report {}: {} in {}", stored.id, reason, name)
}

pub fn body(stored: &StoredReport) -> String {
    format!(
        "Someone reported a community stored on this sync server.\n\n{}\n\nTo remove this community from this sync server, run:\n  commy-sync drop {}\nTo close the report without action:\n  commy-sync dismiss {}\n",
        describe(stored),
        stored.id,
        stored.id
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reports::{Filed, ReportState};
    use commy_sync::report_seal::ReportSecret;
    use commy_sync::report::{HostReport, Person};

    fn stored(community_name: &str) -> StoredReport {
        let person = |id: &str| Person { id: id.into(), name: id.into() };
        StoredReport {
            id: "abc".into(),
            received: 1,
            state: ReportState::Open,
            report: Filed::Plain(HostReport {
                community: "c1".into(),
                community_name: community_name.into(),
                owner: person("o"),
                reporter: person("r"),
                reported: person("t"),
                reason: "spam".into(),
                note: String::new(),
                messages: Vec::new(),
                streams: Vec::new(),
                sent_at: "now".into(),
            }),
        }
    }

    #[test]
    fn the_email_names_the_report_and_how_to_act_on_it() {
        let report = stored("Night\r\nBcc: someone@example.org");
        let subject = subject(&report);
        assert!(!subject.contains('\n') && !subject.contains('\r'), "a community name cannot add mail headers");
        assert!(subject.starts_with("Commy report abc: spam in Night"));
        let body = body(&report);
        assert!(body.contains("commy-sync drop abc"));
        assert!(body.contains("commy-sync dismiss abc"));
    }

    #[test]
    fn a_sealed_report_is_announced_without_its_contents() {
        let Filed::Plain(plain) = stored("Night Owls").report else { panic!("a plain sample") };
        let sealed = StoredReport { report: Filed::Sealed(ReportSecret::generate().public().seal(&plain).unwrap()), ..stored("Night Owls") };
        assert_eq!(subject(&sealed), "Commy report abc: spam (sealed)");
        let body = body(&sealed);
        assert!(!body.contains("Night Owls") && !body.contains("Reported: t"), "the email carries nothing the key protects");
        assert!(body.contains("commy-sync open abc") && body.contains("commy-sync drop abc"));
    }

    #[test]
    fn a_mailer_needs_real_addresses() {
        let settings = SmtpSettings::Host { host: "smtp.example.org".into(), port: 587, user: None, password: None, from: "Commy <no-reply@example.org>".into() };
        assert!(Mailer::new(&settings, "abuse@example.org").is_ok());
        assert!(Mailer::new(&settings, "not an address").is_err());
        let bad_from = SmtpSettings::Host { host: "smtp.example.org".into(), port: 465, user: None, password: None, from: "nobody".into() };
        assert!(Mailer::new(&bad_from, "abuse@example.org").is_err());
    }
}
