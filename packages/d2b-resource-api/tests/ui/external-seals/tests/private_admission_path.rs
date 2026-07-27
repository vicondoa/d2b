use d2b_resource_api::admission::AdmissionIssuer;

fn probe() {
    let _ = core::mem::size_of::<AdmissionIssuer>();
}
