use std::time::{Duration, Instant};

fn churn() {
    for size in [8usize, 16, 24, 32, 48, 64, 96, 128, 256, 1024] {
        for _ in 0..200 {
            drop(vec![0u8; size]);
        }
    }
}

fn run_commy() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = commy_media::start_system_audio(tx).expect("system audio");
    println!("probe: commy capture open");
    churn();
    println!("probe: churn ok");
    let started = Instant::now();
    let mut frames = 0u64;
    while started.elapsed() < Duration::from_secs(15) {
        while rx.try_recv().is_ok() {
            frames += 1;
        }
        churn();
        std::thread::sleep(Duration::from_millis(500));
        println!("probe: {:.1} s, {frames} frames", started.elapsed().as_secs_f32());
    }
    drop(handle);
    println!("probe: commy done");
}

#[cfg(windows)]
fn run_wasapi() {
    use wasapi::{AudioClient, Direction, SampleType, StreamMode, WaveFormat};
    wasapi::initialize_mta().ok().unwrap();
    let format = WaveFormat::new(32, 32, &SampleType::Float, 48000, 2, None);
    let mut client = AudioClient::new_application_loopback_client(std::process::id(), false).expect("activate");
    println!("probe: wasapi activated");
    churn();
    println!("probe: churn ok");
    let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
    client.initialize_client(&format, &Direction::Capture, &mode).expect("initialize");
    println!("probe: wasapi initialized");
    let event = client.set_get_eventhandle().unwrap();
    let capture = client.get_audiocaptureclient().unwrap();
    client.start_stream().unwrap();
    println!("probe: wasapi started");
    let started = Instant::now();
    let mut queue = std::collections::VecDeque::new();
    let mut last = Instant::now();
    while started.elapsed() < Duration::from_secs(15) {
        if event.wait_for_event(500).is_ok() {
            capture.read_from_device_to_deque(&mut queue).unwrap();
        }
        churn();
        if last.elapsed() > Duration::from_millis(500) {
            last = Instant::now();
            println!("probe: {:.1} s, {} bytes", started.elapsed().as_secs_f32(), queue.len());
            queue.clear();
        }
    }
    client.stop_stream().unwrap();
    println!("probe: wasapi done");
}

#[cfg(not(windows))]
fn run_wasapi() {
    println!("probe: wasapi is Windows only");
}

fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();
    match std::env::args().nth(1).as_deref() {
        Some("wasapi") => run_wasapi(),
        _ => run_commy(),
    }
}
