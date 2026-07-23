#[allow(unsafe_code)]
pub fn run(args: libshpool::Args) -> anyhow::Result<()> {
    // libshpool 0.11.0 documents `run` as unsafe because it can initialize
    // global tracing state, daemonize, and exit the process. The helper is the
    // process boundary that contains those effects; guestd never calls this.
    unsafe { libshpool::run(args, None) }
}

#[allow(unsafe_code)]
pub fn run_with_home(args: libshpool::Args, home: &std::path::Path) -> anyhow::Result<()> {
    // The daemon helper is single-threaded before this call and exits by running
    // libshpool; mutating HOME here is the narrow process-boundary effect the
    // helper exists to contain.
    unsafe {
        std::env::set_var("HOME", home);
    }
    run(args)
}
