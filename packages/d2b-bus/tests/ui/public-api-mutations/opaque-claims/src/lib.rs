use d2b_contracts::v3::{ResourceRef, ResourceUid};

pub struct PrincipalClaim(ResourceRef);

impl PrincipalClaim {
    pub fn seal(value: ResourceRef) -> Self {
        Self(value)
    }

    pub fn value(&self) -> &ResourceRef {
        &self.0
    }
}

pub struct SerialClaim(ResourceUid);

impl SerialClaim {
    pub fn seal(value: ResourceUid) -> Self {
        Self(value)
    }

    pub fn value(&self) -> &ResourceUid {
        &self.0
    }
}
