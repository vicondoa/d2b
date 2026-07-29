fn install() {
    mod deep {
        pub mod ordinary {
            pub struct Request;
        }
    }

    use deep::*;

    impl Default for ordinary::Request {
        fn default() -> Self {
            deep::ordinary::Request
        }
    }
}
