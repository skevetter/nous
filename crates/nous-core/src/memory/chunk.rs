use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: String,
    pub memory_id: String,
    pub content: String,
    pub index: usize,
    pub start_offset: usize,
    pub end_offset: usize,
}

pub struct Chunker {
    pub chunk_size: usize,
    pub overlap: usize,
}

impl Default for Chunker {
    fn default() -> Self {
        Self {
            chunk_size: 256,
            overlap: 64,
        }
    }
}

impl Chunker {
    pub fn new(chunk_size: usize, overlap: usize) -> Self {
        Self {
            chunk_size,
            overlap,
        }
    }

    pub fn chunk(&self, memory_id: &str, text: &str) -> Vec<Chunk> {
        if text.is_empty() {
            return Vec::new();
        }

        let tokens: Vec<(usize, &str)> = TokenIterator::new(text).collect();

        if tokens.is_empty() {
            return Vec::new();
        }

        if tokens.len() <= self.chunk_size {
            return vec![Chunk {
                id: format!("{memory_id}_chunk_0"),
                memory_id: memory_id.to_string(),
                content: text.to_string(),
                index: 0,
                start_offset: 0,
                end_offset: text.len(),
            }];
        }

        let step = self.chunk_size.saturating_sub(self.overlap).max(1);
        let mut chunks = Vec::new();
        let mut start_tok = 0;

        while start_tok < tokens.len() {
            let end_tok = (start_tok + self.chunk_size).min(tokens.len());

            let start_offset = tokens[start_tok].0;
            let last = &tokens[end_tok - 1];
            let end_offset = last.0 + last.1.len();

            let content = &text[start_offset..end_offset];

            chunks.push(Chunk {
                id: format!("{memory_id}_chunk_{}", chunks.len()),
                memory_id: memory_id.to_string(),
                content: content.to_string(),
                index: chunks.len(),
                start_offset,
                end_offset,
            });

            start_tok += step;
            if end_tok >= tokens.len() {
                break;
            }
        }

        chunks
    }
}

struct TokenIterator<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> TokenIterator<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, pos: 0 }
    }
}

impl<'a> Iterator for TokenIterator<'a> {
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        let bytes = self.text.as_bytes();

        while self.pos < bytes.len() && bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }

        if self.pos >= bytes.len() {
            return None;
        }

        let start = self.pos;
        while self.pos < bytes.len() && !bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }

        Some((start, &self.text[start..self.pos]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_produces_no_chunks() {
        let chunker = Chunker::default();
        let chunks = chunker.chunk("mem1", "");
        assert!(chunks.is_empty());
    }

    #[test]
    fn text_smaller_than_chunk_size_returns_single_chunk() {
        let chunker = Chunker::default();
        let text = "hello world this is a test";
        let chunks = chunker.chunk("mem1", text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, text);
        assert_eq!(chunks[0].id, "mem1_chunk_0");
        assert_eq!(chunks[0].start_offset, 0);
        assert_eq!(chunks[0].end_offset, text.len());
    }

    #[test]
    fn chunking_with_overlap() {
        let chunker = Chunker::new(4, 2);
        let text = "one two three four five six seven eight";
        let chunks = chunker.chunk("mem1", text);

        assert!(chunks.len() > 1);
        assert_eq!(chunks[0].index, 0);
        assert_eq!(chunks[1].index, 1);

        for chunk in &chunks {
            assert!(chunk.id.starts_with("mem1_chunk_"));
            assert!(!chunk.content.is_empty());
            assert_eq!(chunk.content, &text[chunk.start_offset..chunk.end_offset]);
        }
    }

    #[test]
    fn chunk_ids_are_sequential() {
        let chunker = Chunker::new(3, 1);
        let text = "a b c d e f g h i j";
        let chunks = chunker.chunk("test-id", text);

        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.id, format!("test-id_chunk_{i}"));
            assert_eq!(chunk.index, i);
        }
    }

    #[test]
    fn whitespace_only_text() {
        let chunker = Chunker::default();
        let chunks = chunker.chunk("mem1", "   \n\t  ");
        assert!(chunks.is_empty());
    }

    #[test]
    fn exact_chunk_size_produces_single_chunk() {
        let chunker = Chunker::new(3, 1);
        let text = "one two three";
        let chunks = chunker.chunk("mem1", text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, text);
    }
}
