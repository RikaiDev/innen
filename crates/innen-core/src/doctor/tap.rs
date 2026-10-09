//! `tap_cursor`: the harvest tap can name what it already consumed.

use std::path::Path;

use super::Check;

/// Run the four checks in order and fold the exit code.
///
/// `0` = all pass; `1` = quarantine present OR index stale-but-rebuildable;
/// A tap cursor that cannot name the files it consumed makes the inbox
/// backlog unknowable: the tap must either replay the whole inbox or, with a
/// stale count, silently skip real files. Report it instead of letting
/// `harvest --check` look empty.
pub(super) fn check_tap_cursor(root: &Path) -> Check {
    let state = crate::tap::load_cursor(root, crate::harvest::TAP_ID);
    let detail = match &state {
        crate::tap::CursorState::Missing => {
            return Check {
                name: "tap_cursor".to_string(),
                ok: true,
                detail: "no cursor recorded; inbox is fully unconsumed".to_string(),
            };
        }
        crate::tap::CursorState::Current(cursor) => {
            return Check {
                name: "tap_cursor".to_string(),
                ok: true,
                detail: format!(
                    "current; {} file(s) recorded consumed",
                    cursor.consumed.len()
                ),
            };
        }
        crate::tap::CursorState::LegacyCount(count) => format!(
            "pre-v2 count cursor ({count}) cannot name its inputs; inbox re-lists in full. \
             Run `innen ingest` once to rewrite the cursor as consumed file names."
        ),
        crate::tap::CursorState::Unreadable(why) => format!(
            "cursor unreadable ({why}); inbox re-lists in full. \
             Delete the file or run `innen ingest` once to rewrite it."
        ),
    };
    Check {
        name: "tap_cursor".to_string(),
        ok: false,
        detail,
    }
}
