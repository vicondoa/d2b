#![allow(unsafe_code)]

pub fn run(args: libshpool::Args) -> anyhow::Result<()> {
    // libshpool 0.11.0 documents `run` as unsafe because it can initialize
    // global tracing state, daemonize, and exit the process. The helper is the
    // process boundary that contains those effects; guestd never calls this.
    unsafe { libshpool::run(args, None) }
}
