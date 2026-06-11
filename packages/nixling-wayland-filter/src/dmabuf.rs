use std::{collections::HashMap, os::fd::OwnedFd, rc::Rc, sync::Arc};

use nix::sys::uio::pread;
use wl_proxy::protocols::{
    linux_dmabuf_v1::{
        zwp_linux_dmabuf_feedback_v1::{ZwpLinuxDmabufFeedbackV1, ZwpLinuxDmabufFeedbackV1Handler},
        zwp_linux_dmabuf_v1::{ZwpLinuxDmabufV1, ZwpLinuxDmabufV1Handler},
    },
    wayland::wl_surface::WlSurface,
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
}

struct DmabufFeedbackHandler {
    filters: Arc<DmabufFilterList>,
    table: Option<Vec<u8>>,
}

impl DmabufFeedbackHandler {
    fn new(filters: Arc<DmabufFilterList>) -> Self {
        Self {
            filters,
            table: None,
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
        slf.send_format_table(fd, size);
        self.table = None;
        let mut table = vec![0u8; size as usize];
        let mut offset = 0usize;
        while offset < table.len() {
            match pread(fd.as_ref(), &mut table[offset..], offset as i64) {
                Ok(0) => break,
                Ok(read) => offset += read,
                Err(error) => {
                    log::warn!("dmabuf feedback format table read failed: {error}");
                    return;
                }
            }
        }
        if offset == table.len() {
            self.table = Some(table);
        } else {
            log::warn!(
                "dmabuf feedback format table short read: expected={} got={offset}",
                table.len()
            );
        }
    }

    fn handle_tranche_formats(&mut self, slf: &Rc<ZwpLinuxDmabufFeedbackV1>, indices: &[u8]) {
        let Some(table) = self.table.as_ref() else {
            slf.send_tranche_formats(indices);
            return;
        };
        let Ok(iter) = uapi::pod_iter::<u16, _>(indices) else {
            slf.send_tranche_formats(indices);
            return;
        };
        let mut out = Vec::new();
        for index in iter {
            let offset = usize::from(index) * 16;
            let Some(format_bytes) = table.get(offset..offset + 4) else {
                continue;
            };
            let Some(modifier_bytes) = table.get(offset + 8..offset + 16) else {
                continue;
            };
            let Ok(format) = uapi::pod_read_init::<u32, _>(format_bytes) else {
                continue;
            };
            let Ok(modifier) = uapi::pod_read_init::<u64, _>(modifier_bytes) else {
                continue;
            };
            if self.filters.allowed(format, modifier) {
                out.push(index);
            }
        }
        slf.send_tranche_formats(uapi::as_bytes(out.as_slice()));
    }
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
        assert!(filters.allowed(0x3432_5258, 0x01_0000_0000_000001));
        assert!(filters.allowed(0x3432_5258, INVALID_MODIFIER));
    }

    #[test]
    fn allow_overrides_deny() {
        let format = 0x3432_5258;
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
