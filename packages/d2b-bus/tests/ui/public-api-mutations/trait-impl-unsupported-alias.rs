struct ComponentSessionAdmission;

trait Associated {
    type Output;
}

struct Carrier;

impl Associated for Carrier {
    type Output = ComponentSessionAdmission;
}

type AdmissionAlias = <Carrier as Associated>::Output;

impl Default for AdmissionAlias {
    fn default() -> Self {
        ComponentSessionAdmission
    }
}
