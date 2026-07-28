use d2b_contracts::v3::{ResourceRef, ResourceUid};

pub struct PrincipalClaim(ResourceRef);

impl PrincipalClaim {
    pub fn from_raw(value: &str) -> Option<Self> {
        ResourceRef::parse(value).ok().map(Self)
    }
}

pub struct SerialClaim(ResourceUid);

impl SerialClaim {
    pub fn from_raw(value: &str) -> Option<Self> {
        ResourceUid::parse(value).ok().map(Self)
    }
}
