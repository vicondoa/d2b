// Copyright 2026 The ChromiumOS Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

// Vhost-user backend for virtio-media decoder device.
// Uses VirtioVideoAdapter + VideoDecoder with SHM HostMemoryMapper for MMAP buffers.
pub mod sys;
pub use sys::*;

use std::rc::Rc;
use std::thread;
use anyhow::{anyhow, Context};
use base::{error, Event, WaitContext};
use hypervisor::ProtectionType;
use snapshot::AnySnapshot;
use vm_memory::GuestMemory;
use vmm_vhost::message::VhostUserProtocolFeatures;
use vmm_vhost::VHOST_USER_F_PROTOCOL_FEATURES;

use crate::virtio;
use crate::virtio::copy_config;
use crate::virtio::device_constants::video::VideoBackendType;
use crate::virtio::vhost_user_backend::handler::{DeviceRequestHandler, VhostUserDevice};
use crate::virtio::vhost_user_backend::VhostUserDeviceBuilder;
use crate::virtio::Queue;
use crate::virtio::SharedMemoryMapper;
use crate::virtio::SharedMemoryRegion;
use std::sync::{Arc, Mutex};
use crate::virtio::vhost_user_backend::handler::VhostBackendReqConnection;

const NUM_QUEUES: usize = 2;
const HOST_MAPPER_RANGE: u64 = 256 * 1024 * 1024; // Must match CH media SHM BAR size

pub struct VideoDecoderBackend {
    backend_type: VideoBackendType,
    avail_features: u64,
    config: Vec<u8>,
    pending_queues: [Option<Queue>; NUM_QUEUES],
    pending_mem: Option<GuestMemory>,
    kill_send: Option<Event>,
    worker_thread: Option<thread::JoinHandle<()>>,
    shmem_mapper: Arc<Mutex<Option<Box<dyn SharedMemoryMapper>>>>,
}

impl VideoDecoderBackend {
    pub fn new(backend_type: VideoBackendType) -> anyhow::Result<Self> {
        let mut features = virtio::base_features(ProtectionType::Unprotected);
        features |= 1u64 << VHOST_USER_F_PROTOCOL_FEATURES;

        use virtio_media::v4l2r::ioctl::Capabilities;
        let device_caps = (Capabilities::VIDEO_M2M_MPLANE | Capabilities::STREAMING).bits();
        let mut card = [0u8; 32];
        let name = format!("{backend_type:?} decoder adapter").to_lowercase();
        card[..name.len().min(31)].copy_from_slice(&name.as_bytes()[..name.len().min(31)]);
        let mut config = Vec::with_capacity(40);
        config.extend_from_slice(&device_caps.to_le_bytes());
        config.extend_from_slice(&0u32.to_le_bytes()); // VFL_TYPE_VIDEO
        config.extend_from_slice(&card);

        Ok(Self {
            backend_type,
            avail_features: features,
            config,
            pending_queues: std::array::from_fn(|_| None),
            pending_mem: None,
            kill_send: None,
            worker_thread: None,
            shmem_mapper: Arc::new(Mutex::new(None)),
        })
    }

    fn try_start_worker(&mut self) -> anyhow::Result<()> {
        if self.pending_queues[0].is_none() || self.pending_queues[1].is_none() {
            return Ok(());
        }
        base::info!("Media: both queues ready, starting worker");
        let cmd_q = self.pending_queues[0].take().unwrap();
        let evt_q = self.pending_queues[1].take().unwrap();
        let (kill_s, kill_r) = Event::new()
            .and_then(|e| Ok((e.try_clone()?, e)))
            .context("kill event")?;
        self.kill_send = Some(kill_s);
        let backend = self.backend_type;
        let shmem_mapper = self.shmem_mapper.clone();

        let handle = thread::Builder::new()
            .name("v_media_vhost".into())
            .spawn(move || {
                use crate::virtio::video::decoder::backend::DecoderBackend;
                use crate::virtio::media::decoder_adapter::VirtioVideoAdapter;
                use crate::virtio::media::HostMemoryMapper;
                use virtio_media::devices::video_decoder::VideoDecoder;
                use virtio_media::VirtioMediaDeviceRunner;

                let dec = match backend {
                    #[cfg(feature = "vaapi")]
                    VideoBackendType::Vaapi => match crate::virtio::video::decoder::backend::vaapi::VaapiDecoder::new() {
                        Ok(d) => d.into_trait_object(),
                        Err(e) => { error!("VA-API init failed: {e}"); return; }
                    },
                    #[cfg(feature = "ffmpeg")]
                    VideoBackendType::Ffmpeg => crate::virtio::video::decoder::backend::ffmpeg::FfmpegDecoder::new().into_trait_object(),
                    #[allow(unreachable_patterns)]
                    _ => { error!("Unsupported backend"); return; }
                };

                let eq = crate::virtio::media::EventQueue(evt_q);
                let adapter = VirtioVideoAdapter::new(dec);

                // Try to get the SHM mapper from the backend req connection.
                let mapper_opt = shmem_mapper.lock().unwrap().take();

                if let Some(mapper) = mapper_opt {
                    // SHM mapper available — use real HostMemoryMapper for MMAP buffers.
                    base::info!("Media: SHM mapper available, using HostMemoryMapper for MMAP");
                    use resources::address_allocator::AddressAllocator;
                    use resources::AddressRange;
                    use vm_control::VmMemorySource;

                    // Wrap Box<dyn SharedMemoryMapper> to implement SharedMemoryMapper
                    struct BoxedMapper(Box<dyn crate::virtio::SharedMemoryMapper>);
                    impl crate::virtio::SharedMemoryMapper for BoxedMapper {
                        fn add_mapping(&mut self, source: VmMemorySource, offset: u64, prot: base::Protection, cache: hypervisor::MemCacheType) -> anyhow::Result<()> {
                            self.0.add_mapping(source, offset, prot, cache)
                        }
                        fn remove_mapping(&mut self, offset: u64) -> anyhow::Result<()> {
                            self.0.remove_mapping(offset)
                        }
                        fn as_raw_descriptor(&self) -> Option<base::RawDescriptor> {
                            self.0.as_raw_descriptor()
                        }
                    }

                    let host_mapper = HostMemoryMapper {
                        shm_mapper: BoxedMapper(mapper),
                        allocator: AddressAllocator::new(
                            AddressRange::from_start_and_end(0, HOST_MAPPER_RANGE - 1),
                            Some(base::pagesize() as u64),
                            None,
                        ).unwrap(),
                    };
                    let decoder = VideoDecoder::new(adapter, eq, host_mapper);
                    base::info!("Media decoder created with SHM mapper");
                    run_worker(decoder, cmd_q, kill_r);
                } else {
                    // No SHM mapper — use () (MMAP will fail, only USERPTR would work).
                    base::info!("Media: No SHM mapper, using () — MMAP will fail");
                    let decoder = VideoDecoder::new(adapter, eq, ());
                    base::info!("Media decoder created without SHM");
                    run_worker(decoder, cmd_q, kill_r);
                }
            })
            .context("spawn media worker")?;
        self.worker_thread = Some(handle);
        Ok(())
    }
}

fn run_worker<HM: virtio_media::VirtioMediaHostMemoryMapper + 'static>(
    decoder: virtio_media::devices::video_decoder::VideoDecoder<
        crate::virtio::media::decoder_adapter::VirtioVideoAdapter<
            Box<dyn crate::virtio::video::decoder::backend::DecoderBackend<
                Session = Box<dyn crate::virtio::video::decoder::backend::DecoderSession>,
            >>,
        >,
        crate::virtio::media::EventQueue,
        HM,
    >,
    cmd_q: Queue,
    kill_r: Event,
) {
    use crate::virtio::media::Token;
    use virtio_media::VirtioMediaDevice;
    use virtio_media::VirtioMediaDeviceRunner;

    let wc: Rc<WaitContext<Token>> = Rc::new(WaitContext::<Token>::new().unwrap());
    let _ = wc.add_many(&[(cmd_q.event(), Token::CommandQueue), (&kill_r, Token::Kill)]);

    let mut runner = VirtioMediaDeviceRunner::new(
        decoder,
        crate::virtio::media::WaitContextPoller(Rc::clone(&wc)),
    );
    let mut cq = cmd_q;
    loop {
        let evts = match wc.wait() {
            Ok(e) => e,
            Err(_) => break,
        };
        for e in evts.iter() {
            match e.token {
                Token::CommandQueue => {
                    let _ = cq.event().wait();
                    while let Some(mut d) = cq.pop() {
                        runner.handle_command(&mut d.reader, &mut d.writer);
                        cq.add_used(d);
                        cq.trigger_interrupt();
                    }
                }
                Token::Kill => return,
                Token::V4l2Session(session_id) => {
                    if let Some(session) = runner.sessions.get_mut(&session_id) {
                        if let Err(e) = VirtioMediaDevice::<crate::virtio::Reader, crate::virtio::Writer>::process_events(&mut runner.device, session) {
                            base::error!("session {session_id} process_events: {e}");
                        }
                    }
                }
            }
        }
    }
}

impl VhostUserDeviceBuilder for VideoDecoderBackend {
    fn build(self: Box<Self>, _ex: &cros_async::Executor) -> anyhow::Result<Box<dyn vmm_vhost::Backend>> {
        Ok(Box::new(DeviceRequestHandler::new(*self)))
    }
}

impl VhostUserDevice for VideoDecoderBackend {
    fn max_queue_num(&self) -> usize { NUM_QUEUES }
    fn features(&self) -> u64 { self.avail_features }
    fn protocol_features(&self) -> VhostUserProtocolFeatures {
        VhostUserProtocolFeatures::CONFIG
            | VhostUserProtocolFeatures::MQ
            | VhostUserProtocolFeatures::BACKEND_REQ
            | VhostUserProtocolFeatures::SHMEM_MAP
    }
    fn read_config(&self, offset: u64, data: &mut [u8]) {
        copy_config(data, 0, &self.config, offset);
    }
    fn reset(&mut self) {
        if let Some(k) = self.kill_send.take() { let _ = k.signal(); }
    }
    fn start_queue(&mut self, idx: usize, queue: Queue, _mem: GuestMemory) -> anyhow::Result<()> {
        base::info!("Media: start_queue idx={idx}");
        if idx >= NUM_QUEUES { return Err(anyhow!("bad queue idx")); }
        self.pending_queues[idx] = Some(queue);
        self.try_start_worker()
    }
    fn stop_queue(&mut self, idx: usize) -> anyhow::Result<Queue> {
        if let Some(k) = self.kill_send.take() { let _ = k.signal(); }
        // If the worker is running, join it to ensure clean shutdown.
        if let Some(handle) = self.worker_thread.take() {
            let _ = handle.join();
        }
        self.pending_queues[idx].take().ok_or_else(|| anyhow!("queue not started"))
    }
    fn get_shared_memory_region(&self) -> Option<SharedMemoryRegion> {
        Some(SharedMemoryRegion { id: 0, length: HOST_MAPPER_RANGE })
    }
    fn set_backend_req_connection(&mut self, conn: VhostBackendReqConnection) {
        if let Some(mapper) = conn.shmem_mapper() {
            *self.shmem_mapper.lock().unwrap() = Some(mapper);
            base::info!("Media: SHM mapper received from backend req connection");
        } else {
            base::info!("Media: backend req connection received but no SHM mapper");
        }
    }
    fn enter_suspended_state(&mut self) -> anyhow::Result<()> { Ok(()) }
    fn snapshot(&mut self) -> anyhow::Result<AnySnapshot> { AnySnapshot::to_any(()) }
    fn restore(&mut self, _: AnySnapshot) -> anyhow::Result<()> { Ok(()) }
}
