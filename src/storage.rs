use crate::data::Context;

pub fn show_storage(ctx: &Context) {
    println!("Storage locations:");
    println!("  live Codex auth: {}", ctx.live_auth.display());
    println!("  live PI auth: {}", ctx.pi_auth.display());
    println!("  state directory: {}", ctx.state_dir.display());
    println!(
        "  saved Codex profiles: {}",
        ctx.codex_profiles_dir().display()
    );
    println!("  saved PI profiles: {}", ctx.pi_profiles_dir().display());
    println!(
        "  profile options: {}",
        ctx.profile_options_path().display()
    );
    println!("  account tracker: {}", ctx.tracker_file.display());
}
