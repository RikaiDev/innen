//! Shared tantivy schema + CJK tokenizer (Task 7; full index lands in Task 8a).
//!
//! [`schema`] defines the `body` TEXT field indexed with the `"innen-cjk"`
//! tokenizer and the `id` STORED field. [`ensure_tokenizer`] registers the
//! analyzer on an [`tantivy::Index`]'s tokenizer manager; call it once at
//! index build before indexing — the same `Index` then serves searching
//! (registration is idempotent, manager is shared).
//! tantivy 0.24 ships no jieba tokenizer, so [`JiebaTokenizer`] bridges
//! `jieba-rs` (`cut_for_search`, HMM on) into a [`tantivy::tokenizer`]
//! [`Tokenizer`](tantivy::tokenizer::Tokenizer) with jieba char offsets
//! mapped back to byte offsets.

use std::sync::Arc;

use tantivy::schema::{Schema, TextFieldIndexing, TextOptions, STORED};
use tantivy::tokenizer::{TextAnalyzer, Token, TokenStream, Tokenizer};

/// Tokenizer name referenced by the `body` field's indexing options.
pub const INNEN_CJK: &str = "innen-cjk";

/// jieba-backed tantivy tokenizer behind the `"innen-cjk"` analyzer.
///
/// `Clone` is cheap (shared `Arc` over the loaded dict); `tokenize` uses
/// [`jieba_rs::TokenizeMode::Search`] so CJK queries match sub-word grams
/// the same way at index and query time.
#[derive(Clone, Debug, Default)]
pub struct JiebaTokenizer {
    jieba: Arc<jieba_rs::Jieba>,
}

impl JiebaTokenizer {
    /// Load the embedded jieba dict (no network, no files).
    pub fn new() -> Self {
        Self {
            jieba: Arc::new(jieba_rs::Jieba::new()),
        }
    }
}

/// Pre-tokenized stream produced by [`JiebaTokenizer`].
#[derive(Clone, Debug, Default)]
pub struct JiebaTokenStream {
    tokens: Vec<Token>,
    next_idx: usize,
    current: Token,
}

impl Tokenizer for JiebaTokenizer {
    type TokenStream<'a> = JiebaTokenStream;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> JiebaTokenStream {
        // jieba reports char offsets; tantivy wants byte offsets.
        let mut byte_of_char: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
        byte_of_char.push(text.len());
        let mut tokens = Vec::new();
        for (position, jt) in self
            .jieba
            .tokenize(text, jieba_rs::TokenizeMode::Search, true)
            .iter()
            .enumerate()
        {
            if jt.word.is_empty() {
                continue;
            }
            // Skip out-of-range or degenerate zero-width offsets instead of
            // clamping: emitting a zero-width token would pollute the index.
            let (Some(offset_from), Some(offset_to)) = (
                byte_of_char.get(jt.start).copied(),
                byte_of_char.get(jt.end).copied(),
            ) else {
                continue;
            };
            if offset_from >= offset_to {
                continue;
            }
            tokens.push(Token {
                offset_from,
                offset_to,
                position,
                text: jt.word.to_string(),
                position_length: 1,
            });
        }
        JiebaTokenStream {
            tokens,
            next_idx: 0,
            current: Token::default(),
        }
    }
}

impl TokenStream for JiebaTokenStream {
    fn advance(&mut self) -> bool {
        let Some(next) = self.tokens.get(self.next_idx) else {
            return false;
        };
        self.current = next.clone();
        self.next_idx += 1;
        true
    }

    fn token(&self) -> &Token {
        &self.current
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.current
    }
}

/// Shared schema: `body` TEXT indexed with `"innen-cjk"`, `id` STORED
/// (retrieved from hits, never queried directly; id lookup is the
/// substring seed in [`crate::query`]).
pub fn schema() -> Schema {
    let mut builder = Schema::builder();
    let body_options = TextOptions::default()
        .set_indexing_options(TextFieldIndexing::default().set_tokenizer(INNEN_CJK));
    builder.add_text_field("body", body_options);
    builder.add_text_field("id", STORED);
    builder.build()
}

/// Register (idempotent) the `"innen-cjk"` analyzer on `index`.
/// Call once at index build before indexing; searching reuses the same manager.
pub fn ensure_tokenizer(index: &tantivy::Index) {
    index
        .tokenizers()
        .register(INNEN_CJK, TextAnalyzer::from(JiebaTokenizer::new()));
}
