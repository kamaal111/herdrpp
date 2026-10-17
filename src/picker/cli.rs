use std::{env, path::PathBuf};

use crate::picker::{api::Herdr, config::Config, theme::Theme, ui};
use anyhow::{Context, Result, bail};

pub fn run(api: &Herdr, action: &str) -> Result<()> {
    match action {
        "open" => api.open(),
        "check" => {
            let snapshot = api.snapshot()?;
            println!(
                "Herdr snapshot validated: {} workspaces, {} tabs",
                snapshot.workspaces.len(),
                snapshot.tabs.len()
            );
            Ok(())
        }
        "pick" => {
            let directory = PathBuf::from(
                env::var_os("HERDR_PLUGIN_CONFIG_DIR")
                    .context("launch pick through the Herdr plugin popup")?,
            );
            let config = Config::load(&directory)?;
            let (theme, diagnostics) = Theme::load();
            for diagnostic in diagnostics {
                eprintln!("herdr++: {diagnostic}");
            }
            if let Some((snapshot, target)) = ui::run(api, config.picker.search_mode, theme)? {
                // One focus request validates the target before changing context. Herdr's
                // tab.focus also focuses its parent; no partially completed two-step switch.
                if let Err(error) = api.focus(&target) {
                    let original = snapshot
                        .focused_tab_id
                        .clone()
                        .map(crate::picker::model::Target::Tab)
                        .or_else(|| {
                            snapshot
                                .focused_workspace_id
                                .clone()
                                .map(crate::picker::model::Target::Workspace)
                        });
                    if let Some(original) = original
                        && let Err(restore_error) = api.focus(&original)
                    {
                        bail!("{error:#}; restoring original focus also failed: {restore_error:#}");
                    }
                    return Err(error);
                }
            }
            // The popup closes on process exit, leaving the focused destination intact.
            Ok(())
        }
        _ => bail!("unknown command; use --help"),
    }
}
