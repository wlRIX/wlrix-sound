// SPDX-License-Identifier: GPL-3.0-or-later
//! Playing decoded sounds through PipeWire.
//!
//! Each sound gets a stream of its own, the way libcanberra does it, rather than one long-lived
//! stream mixing everything: a mixer then shows *what* is playing (`media.name` is the sound's
//! name), sounds that overlap are mixed by the graph rather than here, and when nothing is
//! playing there is no node at all. Every stream carries `media.role=Event`, which is what lets
//! a session manager or a mixer treat system sounds apart from music.
//!
//! PipeWire's loop runs on the daemon's calloop loop -- its fd is a source there, the same
//! arrangement as `xdg-desktop-portal-wlrix` -- so the stream callbacks run on the main thread,
//! between the daemon's other sources, and nothing here needs a lock.
//!
//! A stream is never torn down from inside its own callbacks. They only record that the sound
//! has finished; [`Player::reap`] drops finished streams afterwards, from the loop.

use std::cell::RefCell;
use std::os::fd::AsRawFd;
use std::rc::Rc;
use std::time::{Duration, Instant};

use pipewire::context::ContextRc;
use pipewire::core::CoreRc;
use pipewire::main_loop::MainLoopRc;
use pipewire::spa;
use pipewire::stream::{StreamListener, StreamRc, StreamState};

use crate::decode::Pcm;

/// How many sounds may play at once. A burst beyond this -- a drag across a dozen icons, a
/// program calling in a loop -- is dropped rather than queued: a system sound is only worth
/// hearing as it happens.
pub const MAX_VOICES: usize = 4;

/// How long past its own length a sound may take to report it has finished, before it is
/// stopped anyway. A stream with no sink to drain into never reports it.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// How long after a failed connection to PipeWire the next sound tries again. Sounds asked for in
/// between are dropped: a session whose sound server is down should not pay a connection attempt
/// per click.
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

/// What a stream's callbacks share with the player.
struct Progress {
    pcm: Rc<Pcm>,
    /// The next frame to send.
    frame: usize,
    /// Every frame has been sent and the drain requested.
    draining: bool,
    /// Drained, failed or disconnected: the stream can go.
    finished: bool,
}

struct Voice {
    id: u64,
    stream: StreamRc,
    _listener: StreamListener<()>,
    progress: Rc<RefCell<Progress>>,
    deadline: Instant,
}

/// The connection to PipeWire and the sounds playing through it.
pub struct Player {
    main_loop: MainLoopRc,
    context: ContextRc,
    /// `None` until the first sound, and again after the connection breaks.
    core: Option<Connection>,
    /// When a failed connection may be tried again.
    retry_at: Option<Instant>,
    voices: Vec<Voice>,
    next_id: u64,
}

struct Connection {
    core: CoreRc,
    _listener: pipewire::core::Listener,
    /// Set by the core's error callback when the server has gone away.
    broken: Rc<RefCell<bool>>,
}

impl Player {
    /// Set up PipeWire's loop. Does not connect yet: the first sound does that, so a daemon
    /// started before the sound server is not left holding a failed connection.
    pub fn new() -> Result<Self, String> {
        pipewire::init();
        let main_loop =
            MainLoopRc::new(None).map_err(|err| format!("PipeWire main loop: {err}"))?;
        let context =
            ContextRc::new(&main_loop, None).map_err(|err| format!("PipeWire context: {err}"))?;
        Ok(Player {
            main_loop,
            context,
            core: None,
            retry_at: None,
            voices: Vec::new(),
            next_id: 1,
        })
    }

    /// A copy of PipeWire's loop fd, for calloop to watch. calloop's source wants to own its fd,
    /// and the loop keeps its own, so this is a duplicate.
    pub fn loop_fd(&self) -> Result<std::os::fd::OwnedFd, String> {
        let fd = self.main_loop.loop_().fd();
        rustix::io::dup(fd).map_err(|err| {
            format!(
                "could not duplicate the PipeWire loop fd {}: {err}",
                fd.as_raw_fd()
            )
        })
    }

    /// Run whatever PipeWire has ready. Never blocks: calloop has already said there is
    /// something to read, and blocking would stall every other source.
    pub fn iterate(&self) {
        self.main_loop
            .loop_()
            .iterate(pipewire::loop_::Timeout::None);
    }

    /// How many sounds are playing.
    pub fn playing(&self) -> usize {
        self.voices.len()
    }

    /// Start playing `pcm`. Returns the voice's id, which [`Player::reap`] reports when it ends,
    /// or why it could not start.
    pub fn play(&mut self, name: &str, pcm: Rc<Pcm>) -> Result<u64, String> {
        if self.voices.len() >= MAX_VOICES {
            return Err(format!("{MAX_VOICES} sounds are already playing"));
        }
        let core = self.connection()?.core.clone();

        let stream = StreamRc::new(
            core,
            name,
            pipewire::properties::properties! {
                *pipewire::keys::MEDIA_TYPE => "Audio",
                *pipewire::keys::MEDIA_CATEGORY => "Playback",
                *pipewire::keys::MEDIA_ROLE => "Event",
                *pipewire::keys::MEDIA_NAME => name,
                *pipewire::keys::APP_NAME => "wlRIX system sounds",
                *pipewire::keys::APP_ID => "wlrix-sound",
                *pipewire::keys::NODE_NAME => "wlrix-sound",
                // What libcanberra sets, so session policy written for canberra's streams
                // recognizes these.
                "event.id" => name,
            },
        )
        .map_err(|err| format!("could not create a stream: {err}"))?;

        let channels = pcm.channels;
        let rate = pcm.rate;
        let length = Duration::from_secs_f64(pcm.frames() as f64 / f64::from(rate.max(1)));
        let progress = Rc::new(RefCell::new(Progress {
            pcm,
            frame: 0,
            draining: false,
            finished: false,
        }));

        let process_progress = Rc::clone(&progress);
        let drained_progress = Rc::clone(&progress);
        let state_progress = Rc::clone(&progress);
        let listener = stream
            .add_local_listener_with_user_data(())
            .process(move |stream, _| fill(stream, &mut process_progress.borrow_mut()))
            .drained(move |_, _| drained_progress.borrow_mut().finished = true)
            .state_changed(move |_, _, _, new| {
                if matches!(new, StreamState::Error(_) | StreamState::Unconnected) {
                    state_progress.borrow_mut().finished = true;
                }
            })
            .register()
            .map_err(|err| format!("could not listen to the stream: {err}"))?;

        let format = format_pod(rate, channels)?;
        let mut params =
            [spa::pod::Pod::from_bytes(&format).ok_or("could not build the stream format")?];
        stream
            .connect(
                spa::utils::Direction::Output,
                None,
                pipewire::stream::StreamFlags::AUTOCONNECT
                    | pipewire::stream::StreamFlags::MAP_BUFFERS,
                &mut params,
            )
            .map_err(|err| format!("could not connect the stream: {err}"))?;

        let id = self.next_id;
        self.next_id += 1;
        self.voices.push(Voice {
            id,
            stream,
            _listener: listener,
            progress,
            deadline: Instant::now() + length + DRAIN_GRACE,
        });
        Ok(id)
    }

    /// The connection, made now if there is none.
    fn connection(&mut self) -> Result<&Connection, String> {
        if self
            .core
            .as_ref()
            .is_some_and(|connection| *connection.broken.borrow())
        {
            // Dropped with every voice on it: their streams belong to the dead core.
            self.voices.clear();
            self.core = None;
        }
        if self.core.is_none() {
            if self.retry_at.is_some_and(|at| Instant::now() < at) {
                return Err("PipeWire is not running".to_owned());
            }
            match self.connect() {
                Ok(connection) => {
                    self.core = Some(connection);
                    self.retry_at = None;
                }
                Err(err) => {
                    self.retry_at = Some(Instant::now() + RECONNECT_DELAY);
                    return Err(err);
                }
            }
        }
        Ok(self.core.as_ref().expect("connected just above"))
    }

    fn connect(&self) -> Result<Connection, String> {
        let core = self
            .context
            .connect_rc(None)
            .map_err(|err| format!("could not connect to PipeWire: {err}"))?;
        let broken = Rc::new(RefCell::new(false));
        let flag = Rc::clone(&broken);
        let listener = core
            .add_listener_local()
            .error(move |id, _, res, message| {
                // An error on the core object itself, of the kind a server going away produces,
                // is the connection gone. Errors on other objects are a stream's business and
                // reach its state callback.
                if id == pipewire::core::PW_ID_CORE && res == -libc::EPIPE {
                    eprintln!("wlrix-sound: lost PipeWire: {message}");
                    *flag.borrow_mut() = true;
                }
            })
            .register();
        Ok(Connection {
            core,
            _listener: listener,
            broken,
        })
    }

    /// Drop every sound that has finished, or taken too long to, returning their ids.
    pub fn reap(&mut self) -> Vec<u64> {
        let now = Instant::now();
        let broken = self
            .core
            .as_ref()
            .is_some_and(|connection| *connection.broken.borrow());
        let mut done = Vec::new();
        self.voices.retain(|voice| {
            let finished = broken || voice.progress.borrow().finished || now >= voice.deadline;
            if finished {
                // Disconnecting first, rather than leaving it to the drop, so the node leaves the
                // graph now instead of whenever the last reference goes.
                let _ = voice.stream.disconnect();
                done.push(voice.id);
            }
            !finished
        });
        done
    }

    /// When the next sound runs out of time to drain, if one is playing.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.voices.iter().map(|voice| voice.deadline).min()
    }
}

/// Fill the next buffer PipeWire offers, and ask for the drain once everything has been sent.
fn fill(stream: &pipewire::stream::Stream, progress: &mut Progress) {
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let channels = progress.pcm.channels as usize;
    let stride = channels * std::mem::size_of::<f32>();
    let remaining = progress.pcm.frames().saturating_sub(progress.frame);
    let data = &mut buffer.datas_mut()[0];
    let written = match data.data() {
        Some(bytes) => {
            let frames = (bytes.len() / stride).min(remaining);
            let start = progress.frame * channels;
            let samples = &progress.pcm.samples[start..start + frames * channels];
            for (out, sample) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(samples) {
                out.copy_from_slice(&sample.to_le_bytes());
            }
            frames
        }
        None => 0,
    };
    progress.frame += written;
    let chunk = data.chunk_mut();
    *chunk.offset_mut() = 0;
    *chunk.stride_mut() = stride as i32;
    *chunk.size_mut() = (written * stride) as u32;
    // The buffer goes back to PipeWire when it is dropped, at the end of this function.
    if progress.frame >= progress.pcm.frames() && !progress.draining {
        progress.draining = true;
        // `drain` makes PipeWire play out what it already has and then fire `drained`, which is
        // what marks the sound finished. pw-cat ends a file the same way.
        let _ = stream.flush(true);
    }
}

/// The format a sound is offered in: 32-bit float at its own rate and width.
fn format_pod(rate: u32, channels: u32) -> Result<Vec<u8>, String> {
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(rate);
    info.set_channels(channels);
    let mut position = [0; spa::param::audio::MAX_CHANNELS];
    if channels == 1 {
        position[0] = spa::sys::SPA_AUDIO_CHANNEL_MONO;
    } else {
        position[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
        position[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
    }
    info.set_position(position);
    let (cursor, _) = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Format,
            id: spa::sys::SPA_PARAM_EnumFormat,
            properties: info.into(),
        }),
    )
    .map_err(|err| format!("could not build the stream format: {err:?}"))?;
    Ok(cursor.into_inner())
}
