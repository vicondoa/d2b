use d2b_resource_store::StoreSealIdentity;

fn probe(left: StoreSealIdentity, right: StoreSealIdentity) {
    let _ = left == right;
}
