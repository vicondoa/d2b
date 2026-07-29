mod a {
    pub(crate) mod x {
        pub(crate) mod tail {
            pub(crate) struct Request;
        }
    }

    pub(super) use super::b::*;
    pub(crate) use x::tail as y;
}

mod b {
    pub(crate) use super::a::y as x;
}

impl Default for a::x::tail::Request {
    fn default() -> Self {
        Self
    }
}
