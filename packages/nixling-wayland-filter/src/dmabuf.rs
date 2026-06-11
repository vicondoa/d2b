use std::{
    collections::HashMap,
    ffi::CString,
    fs::File,
    io::{self, Seek, SeekFrom, Write},
    os::fd::OwnedFd,
    rc::Rc,
    sync::Arc,
};

use nix::sys::memfd::{memfd_create, MemFdCreateFlag};
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

pub const LINEAR_MODIFIER: u64 = 0;
pub const INVALID_MODIFIER: u64 = 0x00ff_ffff_ffff_ffff;

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
    if s.len() == 4 && s.is_ascii() {
        let mut value = 0u32;
        for &byte in s.as_bytes().iter().rev() {
            value = (value << 8) | u32::from(byte);
        }
        Some(value)
    } else if let Some(hex) = s.strip_prefix("0x") {
        u32::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
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
}

impl DmabufHandler {
    pub fn new(filters: Arc<DmabufFilterList>) -> Self {
        Self { filters }
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
        id.set_handler(DmabufFeedbackHandler::new(self.filters.clone()));
        slf.send_get_default_feedback(id);
    }

    fn handle_get_surface_feedback(
        &mut self,
        slf: &Rc<ZwpLinuxDmabufV1>,
        id: &Rc<ZwpLinuxDmabufFeedbackV1>,
        surface: &Rc<WlSurface>,
    ) {
        id.set_handler(DmabufFeedbackHandler::new(self.filters.clone()));
        slf.send_get_surface_feedback(id, surface);
    }

    fn handle_create_params(
        &mut self,
        slf: &Rc<ZwpLinuxDmabufV1>,
        params_id: &Rc<ZwpLinuxBufferParamsV1>,
    ) {
        params_id.set_handler(DmabufBufferParamsHandler::new(self.filters.clone()));
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
    planes: Vec<DmabufPlane>,
}

impl DmabufBufferParamsHandler {
    fn new(filters: Arc<DmabufFilterList>) -> Self {
        Self {
            filters,
            planes: Vec::new(),
        }
    }

    fn all_planes_allowed(&self, format: u32) -> bool {
        self.planes
            .iter()
            .all(|plane| self.filters.allowed(format, plane.modifier()))
    }

    fn forward_planes(&self, slf: &Rc<ZwpLinuxBufferParamsV1>) {
        for plane in &self.planes {
            slf.send_add(
                &plane.fd,
                plane.plane_idx,
                plane.offset,
                plane.stride,
                plane.modifier_hi,
                plane.modifier_lo,
            );
        }
    }

    fn deny_create(&self, slf: &Rc<ZwpLinuxBufferParamsV1>, format: u32) {
        let denied: Vec<String> = self
            .planes
            .iter()
            .filter(|plane| !self.filters.allowed(format, plane.modifier()))
            .map(|plane| {
                format!(
                    "plane={} modifier=0x{:016x}",
                    plane.plane_idx,
                    plane.modifier()
                )
            })
            .collect();
        log::warn!(
            "dmabuf buffer creation denied: format=0x{format:08x} denied=[{}]",
            denied.join(", ")
        );
        slf.send_failed();
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
        self.planes.push(DmabufPlane {
            fd: fd.clone(),
            plane_idx,
            offset,
            stride,
            modifier_hi,
            modifier_lo,
        });
    }

    fn handle_create(
        &mut self,
        slf: &Rc<ZwpLinuxBufferParamsV1>,
        width: i32,
        height: i32,
        format: u32,
        flags: ZwpLinuxBufferParamsV1Flags,
    ) {
        if self.all_planes_allowed(format) {
            self.forward_planes(slf);
            slf.send_create(width, height, format, flags);
        } else {
            self.deny_create(slf, format);
        }
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
        if self.all_planes_allowed(format) {
            self.forward_planes(slf);
            slf.send_create_immed(buffer_id, width, height, format, flags);
        } else {
            self.deny_create(slf, format);
        }
    }
}

struct DmabufFeedbackHandler {
    filters: Arc<DmabufFilterList>,
    table: Option<Vec<u8>>,
    index_map: Option<Vec<Option<u16>>>,
    table_invalid: bool,
}

impl DmabufFeedbackHandler {
    fn new(filters: Arc<DmabufFilterList>) -> Self {
        Self {
            filters,
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
                    log::warn!("dmabuf feedback format table read failed: {error}");
                    self.table_invalid = true;
                    self.send_empty_format_table(slf);
                    return;
                }
            }
        }
        if offset == table.len() {
            self.send_filtered_format_table(slf, table);
        } else {
            log::warn!(
                "dmabuf feedback format table short read: expected={} got={offset}",
                table.len()
            );
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
            log::warn!("dmabuf feedback tranche_formats malformed; filtering to empty");
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
                log::warn!("dmabuf feedback empty format table creation failed: {error}");
            }
        }
    }

    fn send_filtered_format_table(&mut self, slf: &Rc<ZwpLinuxDmabufFeedbackV1>, table: Vec<u8>) {
        let (filtered, index_map) = filter_format_table(&table, &self.filters);
        if table.len() % 16 != 0 {
            log::warn!(
                "dmabuf feedback format table has trailing {} bytes; dropping incomplete entry",
                table.len() % 16
            );
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
                log::warn!("dmabuf feedback filtered format table creation failed: {error}");
                self.table_invalid = true;
                self.send_empty_format_table(slf);
            }
        }
    }
}

fn filter_format_table(table: &[u8], filters: &DmabufFilterList) -> (Vec<u8>, Vec<Option<u16>>) {
    let mut filtered = Vec::<u8>::new();
    let mut index_map = Vec::<Option<u16>>::new();
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
                log::warn!("dmabuf feedback filtered table exceeds u16 index space at {index}");
                index_map.push(None);
                continue;
            };
            filtered.extend_from_slice(entry);
            index_map.push(Some(new_index));
        } else {
            index_map.push(None);
        }
    }
    (filtered, index_map)
}

fn table_fd(table: &[u8]) -> io::Result<OwnedFd> {
    let name = CString::new("nixling-dmabuf-format-table").expect("static memfd name has no NUL");
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
    fn buffer_params_enforcement_uses_create_format_with_plane_modifiers() {
        let format = 0x3432_5258u32;
        let filters = Arc::new(DmabufFilterList::new(
            &[],
            &[DmabufFilter {
                format: None,
                modifier: Some(LINEAR_MODIFIER),
            }],
        ));
        let mut handler = DmabufBufferParamsHandler::new(filters);
        handler.planes.push(DmabufPlane {
            fd: Rc::new(OwnedFd::from(File::open("/dev/null").unwrap())),
            plane_idx: 0,
            offset: 0,
            stride: 1280,
            modifier_hi: 0,
            modifier_lo: 0,
        });

        assert!(!handler.all_planes_allowed(format));
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

        let (filtered, index_map) = filter_format_table(&table, &filters);

        assert_eq!(filtered.len(), 16);
        assert_eq!(index_map, vec![None, Some(0)]);
        assert_eq!(filtered[0..4], format.to_ne_bytes());
        assert_eq!(filtered[8..16], allowed_modifier.to_ne_bytes());
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
    }
}
