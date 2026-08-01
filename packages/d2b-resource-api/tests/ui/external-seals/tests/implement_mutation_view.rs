use d2b_resource_store_redb::VerifiedMutationView;

fn probe() {
    let _ = core::mem::size_of::<dyn VerifiedMutationView>();
}
