mod cli;
mod dev_install;

fn main() {
    dev_install::handoff_if_stale();
    std::process::exit(cli::run());
}
