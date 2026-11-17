use std::time::{Duration, Instant};

const WARM_UP_PINGS: usize = 2;
const TIMED_PINGS: usize = 6;
const PING_TIMEOUT: Duration = Duration::from_secs(3);

pub async fn warm_round_trip(probe: url::Url) -> Option<Duration> {
    let client = reqwest::Client::builder()
        .timeout(PING_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Commy")
        .build()
        .ok()?;
    let mut timed = Vec::with_capacity(TIMED_PINGS);
    for round in 0..WARM_UP_PINGS + TIMED_PINGS {
        let took = ping_once(&client, &probe).await?;
        if round >= WARM_UP_PINGS {
            timed.push(took);
        }
    }
    median(timed)
}

async fn ping_once(client: &reqwest::Client, probe: &url::Url) -> Option<Duration> {
    let started = Instant::now();
    let response = client.get(probe.clone()).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.bytes().await.ok()?;
    Some(started.elapsed())
}

fn median(mut samples: Vec<Duration>) -> Option<Duration> {
    samples.sort_unstable();
    samples.get(samples.len() / 2).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(values: &[u64]) -> Vec<Duration> {
        values.iter().copied().map(Duration::from_millis).collect()
    }

    #[test]
    fn median_ignores_a_single_slow_wake_up() {
        assert_eq!(median(ms(&[48, 52, 210, 50, 49, 51])), Some(Duration::from_millis(51)));
    }

    #[test]
    fn no_samples_means_no_reading() {
        assert_eq!(median(Vec::new()), None);
    }
}
