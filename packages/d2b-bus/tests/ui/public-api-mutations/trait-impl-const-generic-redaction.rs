struct ComponentSessionAdmission<const N: usize>;

trait Rogue {}

impl Rogue for ComponentSessionAdmission<{ "/home/alice/private/secret.rs".len() }> {}
