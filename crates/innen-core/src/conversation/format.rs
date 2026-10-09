use serde_json::Value;

use super::{attachments, compact, deltas, Page, ReadError};

pub fn format_page(
    page: Page,
    compact: bool,
    deltas: bool,
    attachment_refs: bool,
    attachment: Option<&str>,
    expect_sha256: Option<&str>,
    offset: usize,
) -> Result<Value, ReadError> {
    let ordinary = (compact || attachment_refs)
        .then(|| serde_json::to_value(&page).expect("conversation page serializes"));
    let page = if let Some(pointer) = attachment {
        let hash =
            expect_sha256.ok_or_else(|| ReadError("attachment requires expect_sha256".into()))?;
        attachments::resolve(&page, offset, pointer, hash)?
    } else if attachment_refs {
        attachments::externalize(page, compact)?
    } else if compact {
        compact::encode(&page)
    } else {
        serde_json::to_value(&page).expect("conversation page serializes")
    };
    let guided = if deltas {
        let base = compact::with_guide(page.clone());
        let delta = compact::with_guide(deltas::encode(page));
        if delta.to_string().len() < base.to_string().len() {
            delta
        } else {
            base
        }
    } else {
        compact::with_guide(page)
    };
    let page = match ordinary {
        Some(ordinary) if guided.to_string().len() > ordinary.to_string().len() => ordinary,
        _ => guided,
    };
    Ok(page)
}
