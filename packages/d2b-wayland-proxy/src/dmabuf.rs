use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::CString,
    fs::File,
    io::{self, Seek, SeekFrom, Write},
    os::fd::OwnedFd,
    rc::Rc,
    sync::Arc,
};

use nix::sys::memfd::{MemFdCreateFlag, memfd_create};
use nix::sys::uio::pread;
use wl_proxy::protocols::{
    linux_dmabuf_v1::{
        zwp_linux_buffer_params_v1::{
            ZwpLinuxBufferParamsV1, ZwpLinuxBufferParamsV1Flags, ZwpLinuxBufferParamsV1Handler,
        },
        zwp_linux_dmabuf_feedback_v1::{ZwpLinuxDmabufFeedbackV1, ZwpLinuxDmabufFeedbackV1Handler},
        zwp_linux_dmabuf_v1::{ZwpLinuxDmabufV1, ZwpLinuxDmabufV1Handler},
    },
    wayland::{wl_buffer::WlBuffer, wl_surface::WlSurface},
};

use crate::diag::DiagRateLimiter;

pub const LINEAR_MODIFIER: u64 = 0;
pub const INVALID_MODIFIER: u64 = 0x00ff_ffff_ffff_ffff;
const MAX_DMABUF_PLANES: usize = 4;
const MAX_DENIED_LOG_EXAMPLES: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmabufFilter {
    pub format: Option<u32>,
    pub modifier: Option<u64>,
}

#[derive(Debug, Clone, Default)]
struct DmabufFormatDisposition {
    all_modifiers: DmabufDisposition,
    modifiers: HashMap<u64, DmabufDisposition>,
}

#[derive(Debug, Clone, Default)]
struct DmabufDisposition {
    allow: bool,
    deny_unless_allowed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DmabufFilterList {
    all_formats: DmabufFormatDisposition,
    formats: HashMap<u32, DmabufFormatDisposition>,
}

impl DmabufFilterList {
    pub fn new(allow: &[DmabufFilter], deny: &[DmabufFilter]) -> Self {
        let mut filters = Self::default();
        filters.add(allow, true);
        filters.add(deny, false);
        filters.normalize();
        filters
    }

    pub fn is_empty(&self) -> bool {
        !self.all_formats.all_modifiers.allow
            && !self.all_formats.all_modifiers.deny_unless_allowed
            && self.all_formats.modifiers.is_empty()
            && self.formats.is_empty()
    }

    fn add(&mut self, rules: &[DmabufFilter], allow: bool) {
        for rule in rules {
            let format = match rule.format {
                Some(format) => self.formats.entry(map_shm_to_drm(format)).or_default(),
                None => &mut self.all_formats,
            };
            let disposition = match rule.modifier {
                Some(modifier) => format.modifiers.entry(modifier).or_default(),
                None => &mut format.all_modifiers,
            };
            if allow {
                disposition.allow = true;
            } else {
                disposition.deny_unless_allowed = true;
            }
        }
    }

    fn normalize(&mut self) {
        if self.all_formats.all_modifiers.allow {
            *self = Self::default();
            self.all_formats.all_modifiers.allow = true;
        }
        if self.all_formats.all_modifiers.deny_unless_allowed {
            for format in self.formats.values_mut() {
                format.all_modifiers.deny_unless_allowed = true;
            }
            self.all_formats
                .modifiers
                .retain(|_, disposition| disposition.allow);
            self.all_formats.modifiers.shrink_to_fit();
        }
        for format in self.formats.values_mut() {
            if format.all_modifiers.allow {
                *format = DmabufFormatDisposition::default();
                format.all_modifiers.allow = true;
            }
            if format.all_modifiers.deny_unless_allowed {
                format.modifiers.retain(|_, disposition| disposition.allow);
                format.modifiers.shrink_to_fit();
            }
        }
        self.formats.retain(|_, format| {
            format.all_modifiers.allow
                || (format.all_modifiers.deny_unless_allowed
                    && !self.all_formats.all_modifiers.deny_unless_allowed)
                || !format.modifiers.is_empty()
        });
        self.formats.shrink_to_fit();
    }

    pub fn allowed(&self, format: u32, modifier: u64) -> bool {
        if self.all_formats.all_modifiers.allow {
            return true;
        }
        let mut deny = false;
        if let Some(format) = self.formats.get(&format) {
            if format.all_modifiers.allow {
                return true;
            }
            if let Some(disposition) = format.modifiers.get(&modifier) {
                if disposition.allow {
                    return true;
                }
                deny |= disposition.deny_unless_allowed;
            }
            deny |= format.all_modifiers.deny_unless_allowed;
        }
        if let Some(disposition) = self.all_formats.modifiers.get(&modifier) {
            if disposition.allow {
                return true;
            }
            deny |= disposition.deny_unless_allowed;
        }
        deny |= self.all_formats.all_modifiers.deny_unless_allowed;
        !deny
    }
}

pub fn map_shm_to_drm(format: u32) -> u32 {
    match format {
        0 => 0x3432_5241,
        1 => 0x3432_5258,
        _ => format,
    }
}

pub fn parse_filter(s: &str) -> Result<DmabufFilter, String> {
    let (format, modifier) = match s.split_once(':') {
        Some((format, modifier)) => (format, Some(modifier)),
        None => (s, None),
    };
    let format = if format == "all" {
        None
    } else {
        Some(parse_u32(format).ok_or_else(|| format!("invalid dmabuf format `{format}`"))?)
    };
    let modifier = match modifier {
        None => None,
        Some("linear") => Some(LINEAR_MODIFIER),
        Some("invalid") => Some(INVALID_MODIFIER),
        Some(value) => {
            Some(parse_u64(value).ok_or_else(|| format!("invalid dmabuf modifier `{value}`"))?)
        }
    };
    Ok(DmabufFilter { format, modifier })
}

fn parse_u32(s: &str) -> Option<u32> {
    if let Some(hex) = s.strip_prefix("0x") {
        u32::from_str_radix(hex, 16).ok()
    } else if s.bytes().all(|byte| byte.is_ascii_digit()) {
        s.parse().ok()
    } else if s.len() == 4 && s.is_ascii() {
        let mut value = 0u32;
        for &byte in s.as_bytes().iter().rev() {
            value = (value << 8) | u32::from(byte);
        }
        Some(value)
    } else {
        None
    }
}

fn parse_u64(s: &str) -> Option<u64> {
    if let Some(hex) = s.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

pub struct DmabufHandler {
    filters: Arc<DmabufFilterList>,
    diag: Rc<RefCell<DiagRateLimiter>>,
}

impl DmabufHandler {
    pub fn new(filters: Arc<DmabufFilterList>, diag: Rc<RefCell<DiagRateLimiter>>) -> Self {
        Self { filters, diag }
    }
}

impl ZwpLinuxDmabufV1Handler for DmabufHandler {
    fn handle_format(&mut self, slf: &Rc<ZwpLinuxDmabufV1>, format: u32) {
        if self.filters.allowed(format, INVALID_MODIFIER) {
            slf.send_format(format);
        }
    }

    fn handle_modifier(
        &mut self,
        slf: &Rc<ZwpLinuxDmabufV1>,
        format: u32,
        modifier_hi: u32,
        modifier_lo: u32,
    ) {
        let modifier = (u64::from(modifier_hi) << 32) | u64::from(modifier_lo);
        if self.filters.allowed(format, modifier) {
            slf.send_modifier(format, modifier_hi, modifier_lo);
        }
    }

    fn handle_get_default_feedback(
        &mut self,
        slf: &Rc<ZwpLinuxDmabufV1>,
        id: &Rc<ZwpLinuxDmabufFeedbackV1>,
    ) {
        id.set_handler(DmabufFeedbackHandler::new(
            self.filters.clone(),
            self.diag.clone(),
        ));
        slf.send_get_default_feedback(id);
    }

    fn handle_get_surface_feedback(
        &mut self,
        slf: &Rc<ZwpLinuxDmabufV1>,
        id: &Rc<ZwpLinuxDmabufFeedbackV1>,
        surface: &Rc<WlSurface>,
    ) {
        id.set_handler(DmabufFeedbackHandler::new(
            self.filters.clone(),
            self.diag.clone(),
        ));
        slf.send_get_surface_feedback(id, surface);
    }

    fn handle_create_params(
        &mut self,
        slf: &Rc<ZwpLinuxDmabufV1>,
        params_id: &Rc<ZwpLinuxBufferParamsV1>,
    ) {
        params_id.set_handler(DmabufBufferParamsHandler::new(
            self.filters.clone(),
            self.diag.clone(),
        ));
        slf.send_create_params(params_id);
    }
}

#[derive(Debug, Clone)]
struct DmabufPlane {
    fd: Rc<OwnedFd>,
    plane_idx: u32,
    offset: u32,
    stride: u32,
    modifier_hi: u32,
    modifier_lo: u32,
}

impl DmabufPlane {
    fn modifier(&self) -> u64 {
        (u64::from(self.modifier_hi) << 32) | u64::from(self.modifier_lo)
    }
}

struct DmabufBufferParamsHandler {
    filters: Arc<DmabufFilterList>,
    diag: Rc<RefCell<DiagRateLimiter>>,
    planes: Vec<DmabufPlane>,
    invalid_plane_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DmabufCreateAction {
    Forward,
    Deny {
        denied_count: usize,
        examples: Vec<String>,
    },
}

impl DmabufBufferParamsHandler {
    fn new(filters: Arc<DmabufFilterList>, diag: Rc<RefCell<DiagRateLimiter>>) -> Self {
        Self {
            filters,
            diag,
            planes: Vec::new(),
            invalid_plane_count: 0,
        }
    }

    fn create_action(&self, format: u32) -> DmabufCreateAction {
        if self.invalid_plane_count > 0 {
            return DmabufCreateAction::Deny {
                denied_count: self.invalid_plane_count,
                examples: vec!["invalid plane set".to_string()],
            };
        }
        let denied: Vec<&DmabufPlane> = self
            .planes
            .iter()
            .filter(|plane| !self.filters.allowed(format, plane.modifier()))
            .collect();
        if denied.is_empty() {
            DmabufCreateAction::Forward
        } else {
            let examples = denied
                .iter()
                .take(MAX_DENIED_LOG_EXAMPLES)
                .map(|plane| {
                    let plane = *plane;
                    format!(
                        "plane={} modifier=0x{:016x}",
                        plane.plane_idx,
                        plane.modifier()
                    )
                })
                .collect();
            DmabufCreateAction::Deny {
                denied_count: denied.len(),
                examples,
            }
        }
    }

    fn store_plane(
        &mut self,
        fd: &Rc<OwnedFd>,
        plane_idx: u32,
        offset: u32,
        stride: u32,
        modifier_hi: u32,
        modifier_lo: u32,
    ) {
        if usize::try_from(plane_idx).map_or(true, |idx| idx >= MAX_DMABUF_PLANES) {
            self.invalid_plane_count += 1;
            return;
        }
        if self.planes.iter().any(|plane| plane.plane_idx == plane_idx) {
            self.invalid_plane_count += 1;
            return;
        }
        if self.planes.len() >= MAX_DMABUF_PLANES {
            self.invalid_plane_count += 1;
            return;
        }
        self.planes.push(DmabufPlane {
            fd: fd.clone(),
            plane_idx,
            offset,
            stride,
            modifier_hi,
            modifier_lo,
        });
    }

    fn emit_planes(&self, sink: &mut impl DmabufCreateSink) {
        for plane in &self.planes {
            sink.add(plane);
        }
    }

    fn handle_create_with_sink(
        &self,
        sink: &mut impl DmabufCreateSink,
        width: i32,
        height: i32,
        format: u32,
        flags: ZwpLinuxBufferParamsV1Flags,
    ) {
        match self.create_action(format) {
            DmabufCreateAction::Forward => {
                self.emit_planes(sink);
                sink.create(width, height, format, flags);
            }
            DmabufCreateAction::Deny {
                denied_count,
                examples,
            } => sink.failed(format, denied_count, &examples),
        }
    }

    fn handle_create_immed_with_sink(
        &self,
        sink: &mut impl DmabufCreateSink,
        width: i32,
        height: i32,
        format: u32,
        flags: ZwpLinuxBufferParamsV1Flags,
    ) {
        match self.create_action(format) {
            DmabufCreateAction::Forward => {
                self.emit_planes(sink);
                sink.create_immed(width, height, format, flags);
            }
            DmabufCreateAction::Deny {
                denied_count,
                examples,
            } => sink.failed(format, denied_count, &examples),
        }
    }
}

trait DmabufCreateSink {
    fn add(&mut self, plane: &DmabufPlane);
    fn create(&mut self, width: i32, height: i32, format: u32, flags: ZwpLinuxBufferParamsV1Flags);
    fn create_immed(
        &mut self,
        width: i32,
        height: i32,
        format: u32,
        flags: ZwpLinuxBufferParamsV1Flags,
    );
    fn failed(&mut self, format: u32, denied_count: usize, examples: &[String]);
}

struct ProxyCreateSink<'a> {
    params: &'a Rc<ZwpLinuxBufferParamsV1>,
    buffer: Option<&'a Rc<WlBuffer>>,
    diag: Rc<RefCell<DiagRateLimiter>>,
}

impl DmabufCreateSink for ProxyCreateSink<'_> {
    fn add(&mut self, plane: &DmabufPlane) {
        self.params.send_add(
            &plane.fd,
            plane.plane_idx,
            plane.offset,
            plane.stride,
            plane.modifier_hi,
            plane.modifier_lo,
        );
    }

    fn create(&mut self, width: i32, height: i32, format: u32, flags: ZwpLinuxBufferParamsV1Flags) {
        self.params.send_create(width, height, format, flags);
    }

    fn create_immed(
        &mut self,
        width: i32,
        height: i32,
        format: u32,
        flags: ZwpLinuxBufferParamsV1Flags,
    ) {
        let Some(buffer) = self.buffer else {
            self.diag
                .borrow_mut()
                .warn("dmabuf", "create-immed-missing-buffer", || {
                    "dmabuf create_immed requested without a wl_buffer id".to_owned()
                });
            self.params.send_failed();
            return;
        };
        self.params
            .send_create_immed(buffer, width, height, format, flags);
    }

    fn failed(&mut self, format: u32, denied_count: usize, examples: &[String]) {
        let examples = examples.join(", ");
        self.diag
            .borrow_mut()
            .warn("dmabuf", "buffer-denied", || {
                format!(
                    "dmabuf buffer creation denied: format=0x{format:08x} denied_count={denied_count} examples=[{examples}]"
                )
            });
        self.params.send_failed();
    }
}

impl ZwpLinuxBufferParamsV1Handler for DmabufBufferParamsHandler {
    fn handle_add(
        &mut self,
        _slf: &Rc<ZwpLinuxBufferParamsV1>,
        fd: &Rc<OwnedFd>,
        plane_idx: u32,
        offset: u32,
        stride: u32,
        modifier_hi: u32,
        modifier_lo: u32,
    ) {
        self.store_plane(fd, plane_idx, offset, stride, modifier_hi, modifier_lo);
    }

    fn handle_create(
        &mut self,
        slf: &Rc<ZwpLinuxBufferParamsV1>,
        width: i32,
        height: i32,
        format: u32,
        flags: ZwpLinuxBufferParamsV1Flags,
    ) {
        let mut sink = ProxyCreateSink {
            params: slf,
            buffer: None,
            diag: self.diag.clone(),
        };
        self.handle_create_with_sink(&mut sink, width, height, format, flags);
    }

    fn handle_create_immed(
        &mut self,
        slf: &Rc<ZwpLinuxBufferParamsV1>,
        buffer_id: &Rc<WlBuffer>,
        width: i32,
        height: i32,
        format: u32,
        flags: ZwpLinuxBufferParamsV1Flags,
    ) {
        let mut sink = ProxyCreateSink {
            params: slf,
            buffer: Some(buffer_id),
            diag: self.diag.clone(),
        };
        self.handle_create_immed_with_sink(&mut sink, width, height, format, flags);
    }
}

struct DmabufFeedbackHandler {
    filters: Arc<DmabufFilterList>,
    diag: Rc<RefCell<DiagRateLimiter>>,
    table: Option<Vec<u8>>,
    index_map: Option<Vec<Option<u16>>>,
    table_invalid: bool,
}

impl DmabufFeedbackHandler {
    fn new(filters: Arc<DmabufFilterList>, diag: Rc<RefCell<DiagRateLimiter>>) -> Self {
        Self {
            filters,
            diag,
            table: None,
            index_map: None,
            table_invalid: false,
        }
    }
}

impl ZwpLinuxDmabufFeedbackV1Handler for DmabufFeedbackHandler {
    fn handle_format_table(
        &mut self,
        slf: &Rc<ZwpLinuxDmabufFeedbackV1>,
        fd: &Rc<OwnedFd>,
        size: u32,
    ) {
        self.table = None;
        self.index_map = None;
        self.table_invalid = false;
        if self.filters.is_empty() {
            slf.send_format_table(fd, size);
            return;
        }

        let mut table = vec![0u8; size as usize];
        let mut offset = 0usize;
        while offset < table.len() {
            match pread(fd.as_ref(), &mut table[offset..], offset as i64) {
                Ok(0) => break,
                Ok(read) => offset += read,
                Err(nix::errno::Errno::EINTR) => continue,
                Err(error) => {
                    self.diag.borrow_mut().warn(
                        "dmabuf-feedback",
                        "format-table-read-failed",
                        || format!("dmabuf feedback format table read failed: {error}"),
                    );
                    self.table_invalid = true;
                    self.send_empty_format_table(slf);
                    return;
                }
            }
        }
        if offset == table.len() {
            if !format_table_is_well_formed(&table) {
                let len = table.len();
                self.diag.borrow_mut().warn(
                    "dmabuf-feedback",
                    "format-table-malformed",
                    || {
                        format!(
                            "dmabuf feedback format table has malformed size {len}; filtering to empty"
                        )
                    },
                );
                self.table_invalid = true;
                self.send_empty_format_table(slf);
                return;
            }
            self.send_filtered_format_table(slf, table);
        } else {
            let expected = table.len();
            self.diag
                .borrow_mut()
                .warn("dmabuf-feedback", "format-table-short-read", || {
                    format!(
                        "dmabuf feedback format table short read: expected={expected} got={offset}"
                    )
                });
            self.table_invalid = true;
            self.send_empty_format_table(slf);
        }
    }

    fn handle_tranche_formats(&mut self, slf: &Rc<ZwpLinuxDmabufFeedbackV1>, indices: &[u8]) {
        if self.filters.is_empty() {
            slf.send_tranche_formats(indices);
            return;
        }
        if self.table_invalid {
            slf.send_tranche_formats(&[]);
            return;
        }
        let Some(index_map) = self.index_map.as_ref() else {
            slf.send_tranche_formats(&[]);
            return;
        };
        let Ok(iter) = uapi::pod_iter::<u16, _>(indices) else {
            self.diag
                .borrow_mut()
                .warn("dmabuf-feedback", "tranche-formats-malformed", || {
                    "dmabuf feedback tranche_formats malformed; filtering to empty".to_owned()
                });
            slf.send_tranche_formats(&[]);
            return;
        };
        let mut out = Vec::new();
        for index in iter {
            if let Some(Some(mapped)) = index_map.get(usize::from(index)) {
                out.push(*mapped);
            }
        }
        slf.send_tranche_formats(uapi::as_bytes(out.as_slice()));
    }
}

impl DmabufFeedbackHandler {
    fn send_empty_format_table(&mut self, slf: &Rc<ZwpLinuxDmabufFeedbackV1>) {
        match table_fd(&[]) {
            Ok(fd) => {
                let fd = Rc::new(fd);
                slf.send_format_table(&fd, 0);
            }
            Err(error) => {
                self.diag
                    .borrow_mut()
                    .warn("dmabuf-feedback", "empty-table-create-failed", || {
                        format!("dmabuf feedback empty format table creation failed: {error}")
                    });
            }
        }
    }

    fn send_filtered_format_table(&mut self, slf: &Rc<ZwpLinuxDmabufFeedbackV1>, table: Vec<u8>) {
        let (filtered, index_map, overflowed) = filter_format_table(&table, &self.filters);
        if overflowed {
            self.diag
                .borrow_mut()
                .warn("dmabuf-feedback", "filtered-table-overflow", || {
                    "dmabuf feedback filtered table exceeds u16 index space".to_owned()
                });
        }
        match table_fd(&filtered) {
            Ok(fd) => {
                let size = filtered.len() as u32;
                let fd = Rc::new(fd);
                slf.send_format_table(&fd, size);
                self.table = Some(filtered);
                self.index_map = Some(index_map);
            }
            Err(error) => {
                self.diag.borrow_mut().warn(
                    "dmabuf-feedback",
                    "filtered-table-create-failed",
                    || format!("dmabuf feedback filtered format table creation failed: {error}"),
                );
                self.table_invalid = true;
                self.send_empty_format_table(slf);
            }
        }
    }
}

fn filter_format_table(
    table: &[u8],
    filters: &DmabufFilterList,
) -> (Vec<u8>, Vec<Option<u16>>, bool) {
    let mut filtered = Vec::<u8>::new();
    let mut index_map = Vec::<Option<u16>>::new();
    let mut overflowed = false;
    for (index, entry) in table.chunks_exact(16).enumerate() {
        let Ok(format) = uapi::pod_read_init::<u32, _>(&entry[0..4]) else {
            index_map.push(None);
            continue;
        };
        let Ok(modifier) = uapi::pod_read_init::<u64, _>(&entry[8..16]) else {
            index_map.push(None);
            continue;
        };
        if filters.allowed(format, modifier) {
            let Ok(new_index) = u16::try_from(filtered.len() / 16) else {
                let _ = index;
                overflowed = true;
                index_map.push(None);
                continue;
            };
            filtered.extend_from_slice(entry);
            index_map.push(Some(new_index));
        } else {
            index_map.push(None);
        }
    }
    (filtered, index_map, overflowed)
}

fn format_table_is_well_formed(table: &[u8]) -> bool {
    table.len().is_multiple_of(16)
}

fn table_fd(table: &[u8]) -> io::Result<OwnedFd> {
    let name = CString::new("d2b-dmabuf-format-table").expect("static memfd name has no NUL");
    let fd =
        memfd_create(name.as_c_str(), MemFdCreateFlag::MFD_CLOEXEC).map_err(io::Error::from)?;
    let mut file = File::from(fd);
    file.write_all(table)?;
    file.seek(SeekFrom::Start(0))?;
    Ok(file.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    enum CreateEvent {
        Add {
            plane_idx: u32,
            modifier: u64,
        },
        Create {
            width: i32,
            height: i32,
            format: u32,
            flags: u32,
        },
        CreateImmed {
            width: i32,
            height: i32,
            format: u32,
            flags: u32,
        },
        Failed {
            format: u32,
            denied: Vec<String>,
        },
    }

    #[derive(Default)]
    struct FakeCreateSink {
        events: Vec<CreateEvent>,
    }

    impl DmabufCreateSink for FakeCreateSink {
        fn add(&mut self, plane: &DmabufPlane) {
            self.events.push(CreateEvent::Add {
                plane_idx: plane.plane_idx,
                modifier: plane.modifier(),
            });
        }

        fn create(
            &mut self,
            width: i32,
            height: i32,
            format: u32,
            flags: ZwpLinuxBufferParamsV1Flags,
        ) {
            self.events.push(CreateEvent::Create {
                width,
                height,
                format,
                flags: flags.0,
            });
        }

        fn create_immed(
            &mut self,
            width: i32,
            height: i32,
            format: u32,
            flags: ZwpLinuxBufferParamsV1Flags,
        ) {
            self.events.push(CreateEvent::CreateImmed {
                width,
                height,
                format,
                flags: flags.0,
            });
        }

        fn failed(&mut self, format: u32, _denied_count: usize, examples: &[String]) {
            self.events.push(CreateEvent::Failed {
                format,
                denied: examples.to_vec(),
            });
        }
    }

    fn handler_with_plane(modifier: u64) -> DmabufBufferParamsHandler {
        let filters = Arc::new(DmabufFilterList::new(
            &[],
            &[DmabufFilter {
                format: None,
                modifier: Some(LINEAR_MODIFIER),
            }],
        ));
        let mut handler = DmabufBufferParamsHandler::new(filters, diag());
        handler.planes.push(DmabufPlane {
            fd: Rc::new(OwnedFd::from(File::open("/dev/null").unwrap())),
            plane_idx: 0,
            offset: 0,
            stride: 1280,
            modifier_hi: (modifier >> 32) as u32,
            modifier_lo: modifier as u32,
        });
        handler
    }

    fn diag() -> Rc<RefCell<DiagRateLimiter>> {
        Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())))
    }

    #[test]
    fn deny_linear_filter_blocks_only_linear_by_default() {
        let filters = DmabufFilterList::new(
            &[],
            &[DmabufFilter {
                format: None,
                modifier: Some(LINEAR_MODIFIER),
            }],
        );

        assert!(!filters.allowed(0x3432_5258, LINEAR_MODIFIER));
        assert!(filters.allowed(0x3432_5258, 0x0100_0000_0000_0001));
        assert!(filters.allowed(0x3432_5258, INVALID_MODIFIER));
    }

    #[test]
    fn allow_overrides_deny() {
        let format = 0x3432_5258u32;
        let filters = DmabufFilterList::new(
            &[DmabufFilter {
                format: Some(format),
                modifier: Some(LINEAR_MODIFIER),
            }],
            &[DmabufFilter {
                format: None,
                modifier: Some(LINEAR_MODIFIER),
            }],
        );

        assert!(filters.allowed(format, LINEAR_MODIFIER));
        assert!(!filters.allowed(0x3432_5241, LINEAR_MODIFIER));
    }

    #[test]
    fn buffer_params_create_decision_denies_filtered_modifier() {
        let format = 0x3432_5258u32;
        let handler = handler_with_plane(LINEAR_MODIFIER);

        assert_eq!(
            handler.create_action(format),
            DmabufCreateAction::Deny {
                denied_count: 1,
                examples: vec!["plane=0 modifier=0x0000000000000000".to_string()],
            }
        );
    }

    #[test]
    fn buffer_params_create_decision_forwards_allowed_modifier() {
        let format = 0x3432_5258u32;
        let handler = handler_with_plane(0x0100_0000_0000_0001);

        assert_eq!(handler.create_action(format), DmabufCreateAction::Forward);
    }

    #[test]
    fn denied_create_emits_failed_without_forwarding_planes() {
        let mut sink = FakeCreateSink::default();
        handler_with_plane(LINEAR_MODIFIER).handle_create_with_sink(
            &mut sink,
            1280,
            720,
            0x3432_5258,
            ZwpLinuxBufferParamsV1Flags(0),
        );

        assert_eq!(
            sink.events,
            vec![CreateEvent::Failed {
                format: 0x3432_5258,
                denied: vec!["plane=0 modifier=0x0000000000000000".to_string()],
            }]
        );
    }

    #[test]
    fn allowed_create_forwards_planes_before_create() {
        let mut sink = FakeCreateSink::default();
        handler_with_plane(0x0100_0000_0000_0001).handle_create_with_sink(
            &mut sink,
            1280,
            720,
            0x3432_5258,
            ZwpLinuxBufferParamsV1Flags(0),
        );

        assert_eq!(
            sink.events,
            vec![
                CreateEvent::Add {
                    plane_idx: 0,
                    modifier: 0x0100_0000_0000_0001,
                },
                CreateEvent::Create {
                    width: 1280,
                    height: 720,
                    format: 0x3432_5258,
                    flags: 0,
                },
            ]
        );
    }

    #[test]
    fn allowed_create_immed_forwards_planes_before_create_immed() {
        let mut sink = FakeCreateSink::default();
        handler_with_plane(0x0100_0000_0000_0001).handle_create_immed_with_sink(
            &mut sink,
            1280,
            720,
            0x3432_5258,
            ZwpLinuxBufferParamsV1Flags(0),
        );

        assert_eq!(
            sink.events,
            vec![
                CreateEvent::Add {
                    plane_idx: 0,
                    modifier: 0x0100_0000_0000_0001,
                },
                CreateEvent::CreateImmed {
                    width: 1280,
                    height: 720,
                    format: 0x3432_5258,
                    flags: 0,
                },
            ]
        );
    }

    #[test]
    fn filtered_feedback_table_remaps_allowed_indices() {
        let format = 0x3432_5258u32;
        let allowed_modifier = 0x0100_0000_0000_0001u64;
        let filters = DmabufFilterList::new(
            &[],
            &[DmabufFilter {
                format: None,
                modifier: Some(LINEAR_MODIFIER),
            }],
        );
        let mut table = Vec::new();
        table.extend_from_slice(&format.to_ne_bytes());
        table.extend_from_slice(&0u32.to_ne_bytes());
        table.extend_from_slice(&LINEAR_MODIFIER.to_ne_bytes());
        table.extend_from_slice(&format.to_ne_bytes());
        table.extend_from_slice(&0u32.to_ne_bytes());
        table.extend_from_slice(&allowed_modifier.to_ne_bytes());

        let (filtered, index_map, overflowed) = filter_format_table(&table, &filters);

        assert_eq!(filtered.len(), 16);
        assert!(!overflowed);
        assert_eq!(index_map, vec![None, Some(0)]);
        assert_eq!(filtered[0..4], format.to_ne_bytes());
        assert_eq!(filtered[8..16], allowed_modifier.to_ne_bytes());
    }

    #[test]
    fn malformed_feedback_table_has_no_prefix_entries() {
        let malformed = vec![0u8; 17];

        assert!(!format_table_is_well_formed(&malformed));
    }

    #[test]
    fn parses_fourcc_and_modifier_names() {
        assert_eq!(
            parse_filter("XR24:linear").unwrap(),
            DmabufFilter {
                format: Some(0x3432_5258),
                modifier: Some(LINEAR_MODIFIER),
            }
        );
        assert_eq!(
            parse_filter("all:invalid").unwrap(),
            DmabufFilter {
                format: None,
                modifier: Some(INVALID_MODIFIER),
            }
        );
        assert_eq!(
            parse_filter("0x12").unwrap(),
            DmabufFilter {
                format: Some(0x12),
                modifier: None,
            }
        );
        assert_eq!(
            parse_filter("1234").unwrap(),
            DmabufFilter {
                format: Some(1234),
                modifier: None,
            }
        );
    }

    #[test]
    fn invalid_plane_set_fails_create_without_unbounded_examples() {
        let format = 0x3432_5258u32;
        let filters = Arc::new(DmabufFilterList::new(&[], &[]));
        let mut handler = DmabufBufferParamsHandler::new(filters, diag());
        handler.invalid_plane_count = 100;

        assert_eq!(
            handler.create_action(format),
            DmabufCreateAction::Deny {
                denied_count: 100,
                examples: vec!["invalid plane set".to_string()],
            }
        );
    }
}
