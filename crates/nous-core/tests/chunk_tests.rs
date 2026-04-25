use nous_core::chunk::Chunker;

#[test]
fn short_text_returns_one_chunk() {
    let text = "one two three four five six seven eight nine ten";
    let chunker = Chunker::new(100, 10);
    let chunks = chunker.chunk(text);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].idx, 0);
    assert_eq!(chunks[0].start_char, 0);
    assert_eq!(chunks[0].end_char, text.len());
    assert_eq!(chunks[0].text, text);
}

#[test]
fn long_text_produces_overlapping_chunks() {
    let words: Vec<&str> = (0..100)
        .map(|i| match i % 5 {
            0 => "alpha",
            1 => "bravo",
            2 => "charlie",
            3 => "delta",
            _ => "echo",
        })
        .collect();
    let text = words.join(" ");
    let chunker = Chunker::new(30, 5);
    let chunks = chunker.chunk(&text);

    assert!(
        chunks.len() >= 3,
        "expected at least 3 chunks, got {}",
        chunks.len()
    );

    for chunk in &chunks {
        let word_count = chunk.text.split_whitespace().count();
        assert!(
            word_count >= 5,
            "chunk {} has only {} words",
            chunk.idx,
            word_count
        );
    }

    for pair in chunks.windows(2) {
        let prev_words: Vec<&str> = pair[0].text.split_whitespace().collect();
        let next_words: Vec<&str> = pair[1].text.split_whitespace().collect();
        let overlap_count = prev_words
            .iter()
            .rev()
            .take(5)
            .zip(next_words.iter().take(5))
            .filter(|(a, b)| a == b)
            .count();
        assert!(
            overlap_count > 0,
            "no overlap found between chunk {} and {}",
            pair[0].idx,
            pair[1].idx
        );
    }
}

#[test]
fn below_min_chunk_returns_one_chunk() {
    let text = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty";
    let word_count = text.split_whitespace().count();
    assert_eq!(word_count, 20);
    let chunker = Chunker::new(100, 10);
    let chunks = chunker.chunk(text);
    assert_eq!(chunks.len(), 1);
}

#[test]
fn char_offsets_reconstruct_substrings() {
    let words: Vec<&str> = (0..100)
        .map(|i| match i % 3 {
            0 => "foo",
            1 => "bar",
            _ => "baz",
        })
        .collect();
    let text = words.join(" ");
    let chunker = Chunker::new(30, 5);
    let chunks = chunker.chunk(&text);

    for chunk in &chunks {
        let reconstructed = &text[chunk.start_char..chunk.end_char];
        assert_eq!(
            reconstructed, chunk.text,
            "offset mismatch for chunk {}: expected {:?}, got {:?}",
            chunk.idx, chunk.text, reconstructed
        );
    }
}

#[test]
fn empty_string_returns_empty_vec() {
    let chunker = Chunker::new(100, 10);
    let chunks = chunker.chunk("");
    assert!(chunks.is_empty());
}

#[test]
fn last_small_chunk_merged_into_previous() {
    let words: Vec<String> = (0..35).map(|i| format!("word{i}")).collect();
    let text = words.join(" ");
    // chunk_size=30, overlap=0, min_chunk=32
    // Without merging: chunk0 = 30 words, chunk1 = 5 words (< 32 min)
    // With merging: chunk0 absorbs chunk1 → 1 chunk total
    let mut chunker = Chunker::new(30, 0);
    chunker.min_chunk = 32;
    let chunks = chunker.chunk(&text);
    assert_eq!(
        chunks.len(),
        1,
        "last small chunk should be merged into previous"
    );
    assert_eq!(chunks[0].text, text);
}

#[test]
fn last_chunk_not_merged_when_above_min() {
    let words: Vec<String> = (0..95).map(|i| format!("w{i}")).collect();
    let text = words.join(" ");
    let mut chunker = Chunker::new(30, 0);
    chunker.min_chunk = 5;
    let chunks = chunker.chunk(&text);
    assert!(
        chunks.len() >= 3,
        "expected at least 3 chunks, got {}",
        chunks.len()
    );
    let last = chunks.last().unwrap();
    let last_word_count = last.text.split_whitespace().count();
    assert!(
        last_word_count >= 5,
        "last chunk should not be merged when above min_chunk"
    );
}

#[test]
fn chunk_indices_are_sequential() {
    let words: Vec<String> = (0..100).map(|i| format!("token{i}")).collect();
    let text = words.join(" ");
    let chunker = Chunker::new(20, 3);
    let chunks = chunker.chunk(&text);
    for (i, chunk) in chunks.iter().enumerate() {
        assert_eq!(chunk.idx, i);
    }
}
