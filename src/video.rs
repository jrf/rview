use image::RgbaImage;
use ratatui::layout::Rect;
use std::io;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "mkv", "avi", "webm", "m4v"];
const FRAME_BUFFER_SIZE: usize = 3;
const MAX_DISPLAY_FPS: f64 = 10.0;

pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

struct VideoFrame {
    image: RgbaImage,
    pts: f64,
}

enum VideoCmd {
    Play,
    Pause,
    Stop,
}

pub struct VideoPlayback {
    frame_rx: mpsc::Receiver<VideoFrame>,
    cmd_tx: mpsc::Sender<VideoCmd>,
    pub playing: bool,
    pub current_pts: f64,
    pub duration: f64,
    pub fps: f64,
    pub frame_interval: Duration,
    pub last_frame_time: Instant,
    pub current_frame: Option<RgbaImage>,
    _thread: Option<thread::JoinHandle<()>>,
}

impl VideoPlayback {
    pub fn open(path: &Path, rect: Rect, cell_px: (u32, u32)) -> io::Result<Self> {
        let target_w = (rect.width as u32 * cell_px.0).max(1);
        let target_h = (rect.height as u32 * cell_px.1).max(1);

        let input = ffmpeg::format::input(path)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        let stream = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no video stream found"))?;

        let fps = {
            let rate = stream.avg_frame_rate();
            if rate.denominator() > 0 {
                rate.numerator() as f64 / rate.denominator() as f64
            } else {
                24.0
            }
        };
        let duration = if input.duration() > 0 {
            input.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE)
        } else {
            0.0
        };

        drop(input);

        let (frame_tx, frame_rx) = mpsc::sync_channel(FRAME_BUFFER_SIZE);
        let (cmd_tx, cmd_rx) = mpsc::channel();

        let path_owned = path.to_path_buf();
        let handle = thread::spawn(move || {
            decode_loop(&path_owned, target_w, target_h, &frame_tx, &cmd_rx);
        });

        let display_fps = fps.min(MAX_DISPLAY_FPS);
        let frame_interval = Duration::from_secs_f64(1.0 / display_fps);

        Ok(Self {
            frame_rx,
            cmd_tx,
            playing: true,
            current_pts: 0.0,
            duration,
            fps,
            frame_interval,
            last_frame_time: Instant::now(),
            current_frame: None,
            _thread: Some(handle),
        })
    }

    pub fn poll_frame(&mut self) -> bool {
        if !self.playing {
            return false;
        }

        if self.last_frame_time.elapsed() < self.frame_interval {
            return false;
        }

        match self.frame_rx.try_recv() {
            Ok(frame) => {
                self.current_pts = frame.pts;
                self.current_frame = Some(frame.image);
                self.last_frame_time = Instant::now();
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.playing = false;
                false
            }
        }
    }

    pub fn toggle_pause(&mut self) {
        self.playing = !self.playing;
        if self.playing {
            self.last_frame_time = Instant::now();
            let _ = self.cmd_tx.send(VideoCmd::Play);
        } else {
            let _ = self.cmd_tx.send(VideoCmd::Pause);
        }
    }

    pub fn time_until_next_frame(&self) -> Duration {
        if !self.playing {
            return Duration::from_millis(250);
        }
        let elapsed = self.last_frame_time.elapsed();
        if elapsed >= self.frame_interval {
            Duration::ZERO
        } else {
            self.frame_interval - elapsed
        }
    }

    pub fn stop(&mut self) {
        let _ = self.cmd_tx.send(VideoCmd::Stop);
    }
}

impl Drop for VideoPlayback {
    fn drop(&mut self) {
        self.stop();
    }
}

fn fit_aspect(src_w: u32, src_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if src_w == 0 || src_h == 0 {
        return (max_w, max_h);
    }
    let scale_w = max_w as f64 / src_w as f64;
    let scale_h = max_h as f64 / src_h as f64;
    let scale = scale_w.min(scale_h);
    let w = (src_w as f64 * scale).round() as u32;
    let h = (src_h as f64 * scale).round() as u32;
    (w.max(1), h.max(1))
}

fn decode_loop(
    path: &Path,
    target_w: u32,
    target_h: u32,
    frame_tx: &mpsc::SyncSender<VideoFrame>,
    cmd_rx: &mpsc::Receiver<VideoCmd>,
) {
    let Ok(mut input) = ffmpeg::format::input(path) else {
        return;
    };
    let Some(stream) = input.streams().best(ffmpeg::media::Type::Video) else {
        return;
    };
    let stream_index = stream.index();
    let time_base = stream.time_base();

    let Ok(context) = ffmpeg::codec::context::Context::from_parameters(stream.parameters()) else {
        return;
    };
    let Ok(mut decoder) = context.decoder().video() else {
        return;
    };

    let (fit_w, fit_h) = fit_aspect(decoder.width(), decoder.height(), target_w, target_h);

    let Ok(mut scaler) = ffmpeg::software::scaling::Context::get(
        decoder.format(),
        decoder.width(),
        decoder.height(),
        ffmpeg::util::format::pixel::Pixel::RGBA,
        fit_w,
        fit_h,
        ffmpeg::software::scaling::Flags::BILINEAR,
    ) else {
        return;
    };

    let mut paused = false;

    loop {
        if let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                VideoCmd::Stop => return,
                VideoCmd::Pause => paused = true,
                VideoCmd::Play => paused = false,
            }
        }

        if paused {
            match cmd_rx.recv() {
                Ok(VideoCmd::Play) => paused = false,
                Ok(VideoCmd::Stop) => return,
                Ok(VideoCmd::Pause) => {}
                Err(_) => return,
            }
            continue;
        }

        let mut got_frame = false;
        for (stream, packet) in input.packets() {
            if let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    VideoCmd::Stop => return,
                    VideoCmd::Pause => {
                        paused = true;
                        break;
                    }
                    VideoCmd::Play => paused = false,
                }
            }

            if stream.index() != stream_index {
                continue;
            }

            if decoder.send_packet(&packet).is_err() {
                continue;
            }

            let mut decoded = ffmpeg::util::frame::video::Video::empty();
            while decoder.receive_frame(&mut decoded).is_ok() {
                let mut rgba_frame = ffmpeg::util::frame::video::Video::empty();
                if scaler.run(&decoded, &mut rgba_frame).is_err() {
                    continue;
                }

                if let Some(frame) = extract_frame(&rgba_frame, &decoded, time_base) {
                    if frame_tx.send(frame).is_err() {
                        return;
                    }
                }

                got_frame = true;
            }

            if paused {
                break;
            }
        }

        if paused {
            continue;
        }

        let _ = decoder.send_eof();
        let mut decoded = ffmpeg::util::frame::video::Video::empty();
        while decoder.receive_frame(&mut decoded).is_ok() {
            let mut rgba_frame = ffmpeg::util::frame::video::Video::empty();
            if scaler.run(&decoded, &mut rgba_frame).is_ok() {
                if let Some(frame) = extract_frame(&rgba_frame, &decoded, time_base) {
                    if frame_tx.send(frame).is_err() {
                        return;
                    }
                }
            }
        }

        if input.seek(0, ..).is_err() {
            return;
        }
        decoder.flush();

        if !got_frame {
            return;
        }
    }
}

fn extract_frame(
    rgba_frame: &ffmpeg::util::frame::video::Video,
    decoded: &ffmpeg::util::frame::video::Video,
    time_base: ffmpeg::Rational,
) -> Option<VideoFrame> {
    let w = rgba_frame.width();
    let h = rgba_frame.height();
    let stride = rgba_frame.stride(0);
    let data = rgba_frame.data(0);

    let pixels = if stride == w as usize * 4 {
        data[..w as usize * h as usize * 4].to_vec()
    } else {
        let mut buf = Vec::with_capacity(w as usize * h as usize * 4);
        for row in 0..h as usize {
            let start = row * stride;
            buf.extend_from_slice(&data[start..start + w as usize * 4]);
        }
        buf
    };

    let pts = decoded
        .pts()
        .map(|p| p as f64 * time_base.numerator() as f64 / time_base.denominator() as f64)
        .unwrap_or(0.0);

    RgbaImage::from_raw(w, h, pixels).map(|image| VideoFrame { image, pts })
}

pub fn decode_first_frame(path: &Path, rect: Rect, cell_px: (u32, u32)) -> io::Result<RgbaImage> {
    let target_w = (rect.width as u32 * cell_px.0).max(1);
    let target_h = (rect.height as u32 * cell_px.1).max(1);

    let mut input = ffmpeg::format::input(path)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let stream = input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no video stream"))?;

    let stream_index = stream.index();

    let context =
        ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    let mut decoder = context
        .decoder()
        .video()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let (fit_w, fit_h) = fit_aspect(decoder.width(), decoder.height(), target_w, target_h);

    let mut scaler = ffmpeg::software::scaling::Context::get(
        decoder.format(),
        decoder.width(),
        decoder.height(),
        ffmpeg::util::format::pixel::Pixel::RGBA,
        fit_w,
        fit_h,
        ffmpeg::software::scaling::Flags::BILINEAR,
    )
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    for (stream, packet) in input.packets() {
        if stream.index() != stream_index {
            continue;
        }
        if decoder.send_packet(&packet).is_err() {
            continue;
        }
        let mut decoded = ffmpeg::util::frame::video::Video::empty();
        if decoder.receive_frame(&mut decoded).is_ok() {
            let mut rgba_frame = ffmpeg::util::frame::video::Video::empty();
            scaler
                .run(&decoded, &mut rgba_frame)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

            let w = rgba_frame.width();
            let h = rgba_frame.height();
            let stride = rgba_frame.stride(0);
            let data = rgba_frame.data(0);

            let pixels = if stride == w as usize * 4 {
                data[..w as usize * h as usize * 4].to_vec()
            } else {
                let mut buf = Vec::with_capacity(w as usize * h as usize * 4);
                for row in 0..h as usize {
                    let start = row * stride;
                    buf.extend_from_slice(&data[start..start + w as usize * 4]);
                }
                buf
            };

            return RgbaImage::from_raw(w, h, pixels)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid frame"));
        }
    }

    Err(io::Error::new(io::ErrorKind::InvalidData, "no frames decoded"))
}
