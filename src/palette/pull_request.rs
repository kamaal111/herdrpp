use super::catalog::{Command, Context, Item};
use serde_json::Value;

pub fn item_from_snapshot(snapshot: &Value, context: &Context) -> Option<Item> {
    let workspace_id = context.workspace_id.as_deref()?;
    let workspace = snapshot["workspaces"]
        .as_array()?
        .iter()
        .find(|workspace| workspace["workspace_id"].as_str() == Some(workspace_id))?;
    let digits = workspace["tokens"][crate::pr::herdr::TOKEN]
        .as_str()?
        .strip_prefix('#')?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let number: u64 = digits.parse().ok().filter(|number| *number > 0)?;
    let directory = super::repository::directory_from_snapshot(snapshot, context)?;
    Item::new(
        "herdr:view_pull_request".into(),
        String::new(),
        "Herdr".into(),
        String::new(),
        Command::ViewPullRequest { number, directory },
    )
    .ok()
}
