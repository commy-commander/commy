use crate::devices::{AudioStream, FrameAssembler, StreamFault};
use anyhow::{anyhow, Context, Result};
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use windows::core::{implement, Interface, Ref, HRESULT};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
    IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient, IAudioClient, AUDCLNT_BUFFERFLAGS_SILENT,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX,
    WAVE_FORMAT_PCM,
};
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, BLOB, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::Variant::VT_BLOB;

const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
const BUFFER_100NS: i64 = 200_000;
const ACTIVATION_WAIT: Duration = Duration::from_secs(5);
const WAKE_MS: u32 = 200;

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Activated(std::sync::mpsc::SyncSender<()>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
    fn ActivateCompleted(&self, _: Ref<IActivateAudioInterfaceAsyncOperation>) -> windows::core::Result<()> {
        let _ = self.0.try_send(());
        Ok(())
    }
}

fn client_without_this_app() -> Result<IAudioClient> {
    let params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: std::process::id(),
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    let blob = ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: &params as *const AUDIOCLIENT_ACTIVATION_PARAMS as *mut u8,
                    },
                },
            }),
        },
    });
    let (done, finished) = sync_channel(1);
    let handler: IActivateAudioInterfaceCompletionHandler = Activated(done).into();
    let operation = unsafe {
        ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, &IAudioClient::IID, Some(&*blob), &handler)
    }
    .context("this version of Windows cannot leave one app out of a capture")?;
    finished.recv_timeout(ACTIVATION_WAIT).context("Windows did not open the capture in time")?;
    let mut result = HRESULT(0);
    let mut activated = None;
    unsafe { operation.GetActivateResult(&mut result, &mut activated) }?;
    result.ok()?;
    Ok(activated.ok_or_else(|| anyhow!("Windows opened no capture"))?.cast()?)
}

struct Capture {
    client: IAudioClient,
    reader: IAudioCaptureClient,
    wake: HANDLE,
}

impl Drop for Capture {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
            let _ = CloseHandle(self.wake);
        }
    }
}

impl Capture {
    fn open() -> Result<Self> {
        let client = client_without_this_app()?;
        let block = CHANNELS * 2;
        let format = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM as u16,
            nChannels: CHANNELS,
            nSamplesPerSec: RATE,
            nAvgBytesPerSec: RATE * u32::from(block),
            nBlockAlign: block,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        let flags = AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM;
        unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, BUFFER_100NS, 0, &format, None) }?;
        let wake = unsafe { CreateEventW(None, false, false, None) }?;
        let capture = Self {
            reader: unsafe { client.GetService() }?,
            client,
            wake,
        };
        unsafe { capture.client.SetEventHandle(capture.wake) }?;
        unsafe { capture.client.Start() }?;
        Ok(capture)
    }

    fn read(&self, samples: &mut Vec<f32>) -> windows::core::Result<()> {
        samples.clear();
        loop {
            if unsafe { self.reader.GetNextPacketSize() }? == 0 {
                return Ok(());
            }
            let mut data = std::ptr::null_mut();
            let mut frames = 0u32;
            let mut flags = 0u32;
            unsafe { self.reader.GetBuffer(&mut data, &mut frames, &mut flags, None, None) }?;
            let count = frames as usize * usize::from(CHANNELS);
            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                samples.extend(std::iter::repeat_n(0.0, count));
            } else {
                let pcm = unsafe { std::slice::from_raw_parts(data as *const i16, count) };
                samples.extend(pcm.iter().map(|s| f32::from(*s) / 32768.0));
            }
            unsafe { self.reader.ReleaseBuffer(frames) }?;
        }
    }
}

pub struct ProcessLoopback {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for ProcessLoopback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl AudioStream for ProcessLoopback {
    fn start(&self) -> Result<()> {
        Ok(())
    }
}

impl ProcessLoopback {
    pub fn open(tx: UnboundedSender<Vec<i16>>, fault: StreamFault) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let (ready, opened) = sync_channel(1);
        let thread = std::thread::spawn(move || {
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let capture = match Capture::open() {
                Ok(capture) => {
                    let _ = ready.send(Ok(()));
                    capture
                }
                Err(e) => {
                    let _ = ready.send(Err(e));
                    unsafe { CoUninitialize() };
                    return;
                }
            };
            let mut assembler = FrameAssembler::new(RATE, CHANNELS);
            let mut samples = Vec::new();
            while !stopped.load(Ordering::Relaxed) {
                if unsafe { WaitForSingleObject(capture.wake, WAKE_MS) } != WAIT_OBJECT_0 {
                    continue;
                }
                if let Err(e) = capture.read(&mut samples) {
                    tracing::error!("shared audio capture error: {e}");
                    fault.raise();
                    break;
                }
                assembler.push(&samples);
                while let Some(frame) = assembler.next_frame() {
                    let _ = tx.send(crate::devices::pcm16(&frame));
                }
            }
            drop(capture);
            unsafe { CoUninitialize() };
        });
        opened.recv().context("shared audio thread died")??;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_a_capture_that_leaves_this_app_out_keeps_the_heap_intact() {
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let _ = client_without_this_app();
        let survivors: Vec<Vec<u8>> = (0..1000).map(|n| vec![0; n % 97 + 1]).collect();
        drop(survivors);
        unsafe { CoUninitialize() };
    }
}
