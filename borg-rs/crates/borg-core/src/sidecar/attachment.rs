use std::path::Path;

use anyhow::Result;
use tokio::fs;

use super::SidecarAttachment;

pub(crate) async fn save_bytes(
    bytes: &[u8],
    source: &str,
    filename: &str,
    content_type: &str,
    data_dir: &Path,
) -> Result<SidecarAttachment> {
    let dir = data_dir.join("attachments").join(source);
    fs::create_dir_all(&dir).await?;

    let safe_name = sanitize_filename(filename);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let dest_name = format!("{ts}_{safe_name}");
    let dest = dir.join(&dest_name);

    fs::write(&dest, bytes).await?;

    Ok(SidecarAttachment {
        url: dest.to_string_lossy().to_string(),
        filename: safe_name,
        content_type: content_type.to_string(),
    })
}

fn sanitize_filename(name: &str) -> String {
    let name = name.replace(['/', '\\', '\0'], "_");
    if name.is_empty() {
        "attachment".to_string()
    } else {
        name
    }
}
