// Prevents additional console window on Windows in release, do not remove!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(feature = "tracy")]
#[global_allocator]
static GLOBAL: tracy_client::ProfiledAllocator<std::alloc::System> =
    tracy_client::ProfiledAllocator::new(std::alloc::System, 100);
fn main() {
    if std::env::args().any(|arg| arg == "--run-job-worker") {
        preview_alternative_lib::job_worker::run_job_worker();
        return;
    }
    if std::env::args().any(|arg| arg == "--run-worker") {
        preview_alternative_lib::worker::run_worker();
        return;
    }
    if std::env::args().any(|arg| arg == "--run-plugin-worker") {
        preview_alternative_lib::plugins::worker::run_plugin_worker();
        return;
    }
    if std::env::args().any(|arg| arg == "--smoke") {
        preview_alternative_lib::run_smoke();
        return;
    }
    preview_alternative_lib::run();
}
