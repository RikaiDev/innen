//! The Codex delivery-phase marker must survive dialogue projection: a final
//! answer is not interchangeable with an intermediate step.

use super::dialogue::dialogue;
use crate::conversation::sources::Source;
use serde_json::json;

#[test]
fn codex_final_answer_phase_survives_dialogue_projection() {
    let event = json!({
        "type": "response_item",
        "payload": {
            "type": "message",
            "role": "assistant",
            "phase": "final_answer",
            "content": [{"type": "output_text", "text": "[quote](</tmp/quote.docx>)"}]
        }
    });
    let projected = dialogue(Source::Codex, event).unwrap().unwrap();
    assert_eq!(projected["phase"], "final_answer");
    assert_eq!(projected["role"], "assistant");
}
