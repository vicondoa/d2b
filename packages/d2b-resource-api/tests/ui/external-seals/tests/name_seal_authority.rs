use d2b_resource_store::mutation_seal::authority::SealAuthority;

fn probe() {
    let _ = core::mem::size_of::<SealAuthority>();
}
