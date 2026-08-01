use d2b_resource_store::SealedMutation;
use d2b_resource_store::mutation_seal::MutationSealAcceptor;

fn probe(acceptor: MutationSealAcceptor, sealed: SealedMutation) {
    let _ = acceptor.open(sealed);
    let _ = acceptor.open(sealed);
}
