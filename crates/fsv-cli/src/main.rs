#[cfg(unix)]
extern "C" fn on_sigint(_: libc::c_int) {
    // Ctrl-C exits 130 silently, like the Python entry point.
    unsafe { libc::_exit(130) }
}

/// Port of fsv.__init__.main.
fn main() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGINT, on_sigint as *const () as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    std::process::exit(fsv_cli::run(std::env::args().collect()));
}
