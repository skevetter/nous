pub struct Chunk {
    pub idx: usize,
    pub start_char: usize,
    pub end_char: usize,
    pub text: String,
}

pub struct Chunker {
    pub chunk_size: usize,
    pub chunk_overlap: usize,
    pub min_chunk: usize,
}

impl Chunker {
    pub fn new(chunk_size: usize, chunk_overlap: usize) -> Self {
        Self {
            chunk_size,
            chunk_overlap,
            min_chunk: 32,
        }
    }

    pub fn chunk(&self, text: &str) -> Vec<Chunk> {
        if text.is_empty() {
            return Vec::new();
        }

        let tokens: Vec<(usize, usize)> = self.token_spans(text);
        if tokens.is_empty() {
            return Vec::new();
        }

        if tokens.len() <= self.chunk_size {
            return vec![Chunk {
                idx: 0,
                start_char: 0,
                end_char: text.len(),
                text: text.to_string(),
            }];
        }

        let step = self.chunk_size.saturating_sub(self.chunk_overlap).max(1);
        let mut chunks = Vec::new();
        let mut start = 0;

        while start < tokens.len() {
            let end = (start + self.chunk_size).min(tokens.len());
            let start_char = tokens[start].0;
            let end_char = tokens[end - 1].1;
            chunks.push(Chunk {
                idx: chunks.len(),
                start_char,
                end_char,
                text: text[start_char..end_char].to_string(),
            });
            if end >= tokens.len() {
                break;
            }
            start += step;
        }

        if chunks.len() >= 2 {
            let last = chunks.last().unwrap();
            let last_word_count = last.text.split_whitespace().count();
            if last_word_count < self.min_chunk {
                let last_end = chunks.last().unwrap().end_char;
                chunks.pop();
                let prev = chunks.last_mut().unwrap();
                prev.end_char = last_end;
                prev.text = text[prev.start_char..prev.end_char].to_string();
            }
        }

        chunks
    }

    fn token_spans(&self, text: &str) -> Vec<(usize, usize)> {
        let mut spans = Vec::new();
        let mut chars = text.char_indices().peekable();
        while let Some(&(i, c)) = chars.peek() {
            if c.is_whitespace() {
                chars.next();
                continue;
            }
            let start = i;
            let mut end = i;
            while let Some(&(j, ch)) = chars.peek() {
                if ch.is_whitespace() {
                    break;
                }
                end = j + ch.len_utf8();
                chars.next();
            }
            spans.push((start, end));
        }
        spans
    }
}
