use std::io::Cursor;
use std::{
    io::BufWriter,
    sync::{
        Arc,
        LazyLock, // Changed from once_cell::sync::Lazy
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam::channel::{Receiver, Sender};
// Removed: use once_cell::sync::Lazy;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use snafu::{Location, ResultExt, Snafu};
use tracing::error;
use whisper_rs::{
    FullParams, GGMLLogLevel, SamplingStrategy, WhisperContext, WhisperContextParameters,
};

use crate::{config::ParsedConfig, worker};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Sending event to worker"))]
    SendEvent {
        #[snafu(implicit)]
        location: Location,
        #[snafu(source)]
        source: crossbeam::channel::SendError<worker::Event>,
    },

    #[snafu(whatever, display("{message}"))]
    Whatever {
        message: String,
        #[snafu(source(from(Box<dyn std::error::Error + Send + Sync>, Some)))]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

pub type MResult<T> = Result<T, Error>;

/// Tasks the microphone can receive from the worker
#[derive(Debug, Clone)]
pub enum Task {
    ToggleRecord,
}

// Define the model path as a constant
const MODEL_PATH: &str = "/Users/silasmarvin/github/copilot/models/ggml-tiny.bin";

// Catch whipser logs so we don't just spam stderr
extern "C" fn silence_whisper_logger(
    level: std::ffi::c_uint,
    text: *const std::ffi::c_char,
    _user_data: *mut std::ffi::c_void,
) {
    let log_level = GGMLLogLevel::from(level as u32);
    unsafe {
        match log_level {
            GGMLLogLevel::Error => {
                let c_str = std::ffi::CStr::from_ptr(text);
                eprintln!("[Whisper C Error]: {:?}", c_str);
            }
            _ => (),
        }
    }
}

// Lazily initialize WhisperContext so it's loaded only once using std::sync::LazyLock
static WHISPER_CONTEXT: LazyLock<WhisperContext> = LazyLock::new(|| {
    // Set the log callback to our no-op function *before* initializing the context.
    // This affects global logging from the underlying whisper.cpp library.
    unsafe {
        whisper_rs::set_log_callback(Some(silence_whisper_logger), std::ptr::null_mut());
    }

    WhisperContext::new_with_params(MODEL_PATH, WhisperContextParameters::default())
        .expect("failed to load model")
});

pub fn execute_microphone(tx: Sender<worker::Event>, rx: Receiver<Task>, config: ParsedConfig) {
    if let Err(e) = do_execute_microphone(tx, rx, config) {
        error!("Error while executing microphone: {e:?}");
    }
}

fn do_execute_microphone(
    tx: Sender<worker::Event>,
    rx: Receiver<Task>,
    _config: ParsedConfig,
) -> MResult<()> {
    let recording = Arc::new(AtomicBool::new(false));

    while let Ok(task) = rx.recv() {
        match task {
            Task::ToggleRecord => {
                if !recording.load(Ordering::Relaxed) {
                    let local_recording = recording.clone();
                    let local_tx = tx.clone();
                    thread::spawn(move || {
                        if let Err(e) = record_audio(local_recording, local_tx) {
                            error!("Error while recording audio: {e:?}");
                        }
                    });
                    recording.store(true, Ordering::Relaxed);
                } else {
                    recording.store(false, Ordering::Relaxed);
                }
            }
        }
    }

    Ok(())
}

// TODO: There is a bunch todo here.
// We should not assume the default config is f32, we should iterate over configs and see if they
// support it. We should also have support for converting the int audio to f32. Really, this
// section just needs a whole review
fn record_audio(recording: Arc<AtomicBool>, tx: Sender<worker::Event>) -> MResult<()> {
    let host = cpal::default_host();
    let device = host.default_input_device().unwrap();
    let config = device.default_input_config().unwrap();
    let sample_rate = config.sample_rate().0;

    // We'll use WAV format, which is widely supported
    let spec = hound::WavSpec {
        channels: config.channels() as u16,
        sample_rate: config.sample_rate().0,
        bits_per_sample: match config.sample_format() {
            cpal::SampleFormat::F32 => 32,
            cpal::SampleFormat::I16 => 16,
            cpal::SampleFormat::U16 => 16,
            _ => {
                return Err(Error::Whatever {
                    message: "Unsupported bits_per_sample".to_string(),
                    source: None,
                });
            }
        },
        sample_format: match config.sample_format() {
            cpal::SampleFormat::F32 => hound::SampleFormat::Float,
            cpal::SampleFormat::I16 | cpal::SampleFormat::U16 => hound::SampleFormat::Int,
            _ => {
                return Err(Error::Whatever {
                    message: "Unsupported config sample_format".to_string(),
                    source: None,
                });
            }
        },
    };

    let mut bufwriter = BufWriter::new(Cursor::new(Vec::new()));
    let static_bufwriter = unsafe {
        std::mem::transmute::<
            &mut BufWriter<Cursor<Vec<u8>>>,
            &'static mut BufWriter<Cursor<Vec<u8>>>,
        >(&mut bufwriter)
    };
    let writer = hound::WavWriter::new(static_bufwriter, spec).unwrap();
    let writer = Arc::new(std::sync::Mutex::new(Some(writer)));

    let writer_clone = writer.clone();
    let err_fn = |err| eprintln!("An error occurred on stream: {}", err);

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device
            .build_input_stream(
                &config.into(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if let Some(writer) = writer_clone.lock().unwrap().as_mut() {
                        for &sample in data {
                            writer.write_sample(sample).unwrap();
                        }
                    }
                },
                err_fn,
                None,
            )
            .unwrap(),
        cpal::SampleFormat::I16 => device
            .build_input_stream(
                &config.into(),
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    if let Some(writer) = writer_clone.lock().unwrap().as_mut() {
                        for &sample in data {
                            writer.write_sample(sample).unwrap();
                        }
                    }
                },
                err_fn,
                None,
            )
            .unwrap(),
        _ => {
            return Err(Error::Whatever {
                message: "Unspported cpal SampleFormat".to_string(),
                source: None,
            });
        }
    };

    stream.play().unwrap();
    while recording.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }

    // Stop recording
    drop(stream); // Stream must be dropped before writer is finalized
    writer.lock().unwrap().take().unwrap().finalize().unwrap();

    // Get the raw bytes from the WAV writer
    let raw_audio_bytes: Vec<u8> = bufwriter
        .into_inner()
        .map_err(|e| Error::Whatever {
            message: format!("Failed to get inner from BufWriter: {:?}", e),
            source: None,
        })?
        .into_inner();

    let mut wav_reader = hound::WavReader::new(Cursor::new(raw_audio_bytes.clone())).unwrap();

    let audio_f32_maybe_stereo: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => wav_reader
            .samples::<f32>()
            .map(|s_res| s_res.expect("Failed to read f32 sample from WavReader"))
            .collect(),
        _ => {
            return Err(Error::Whatever {
                message: format!(
                    "Unsupported WAV spec for whisper.rs processing: format {:?}, bits {}",
                    spec.sample_format, spec.bits_per_sample
                ),
                source: None,
            });
        }
    };

    // At this point, audio_f32_maybe_stereo contains the audio data as f32 samples.
    // Now, handle stereo to mono conversion if necessary.
    // whisper-rs expects mono audio.
    let mono_samples: Vec<f32> = if spec.channels > 1 {
        // Your spec.channels comes from config.channels()
        if spec.channels == 2 {
            // Common case: stereo
            whisper_rs::convert_stereo_to_mono_audio(&audio_f32_maybe_stereo)
                .expect("failed to convert stereo to mono audio")
        } else {
            // For more than 2 channels, you might need custom logic to mix down to mono.
            // convert_stereo_to_mono_audio likely just averages the first two.
            // For simplicity, let's assume it handles it or we warn.
            eprintln!(
                "Warning: {} channels found, whisper_rs::convert_stereo_to_mono_audio will be used. It might only process the first two.",
                spec.channels
            );
            whisper_rs::convert_stereo_to_mono_audio(&audio_f32_maybe_stereo)
                .expect("failed to convert multi-channel to mono audio")
        }
    } else {
        audio_f32_maybe_stereo // Already mono (or effectively treated as such)
    };

    let sync_params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };
    let mut resampler = SincFixedIn::<f32>::new(
        16000 as f64 / sample_rate as f64,
        2.0,
        sync_params,
        mono_samples.len(),
        1,
    )
    .unwrap();

    let samples_for_whisper = resampler.process(&vec![mono_samples], None).unwrap();

    // Use the statically initialized WhisperContext
    // The &* dereferences the LazyLock<WhisperContext> to &WhisperContext
    let ctx = &*WHISPER_CONTEXT;

    let mut state = ctx.create_state().expect("failed to create state");
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

    let language = "en";
    params.set_language(Some(&language));

    // Disable anything that prints to stdout
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    // Run the model
    state
        .full(params, &samples_for_whisper[0][..])
        .expect("failed to run model");

    // Get the transcribed text
    let num_segments = state
        .full_n_segments()
        .expect("Failed to get number of segments");
    // Send the transcribed text (example: join all segments)
    let mut full_text = String::new();
    for i in 0..num_segments {
        full_text.push_str(&state.full_get_segment_text(i).expect("msg"));
        if i < num_segments - 1 {
            full_text.push(' ');
        }
    }

    if !full_text.is_empty() {
        tx.send(worker::Event::MicrophoneResponse(full_text))
            .context(SendEventSnafu)?;
    }

    Ok(())
}
