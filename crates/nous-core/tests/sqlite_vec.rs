use rusqlite::Connection;

fn vec0_knn_round_trip_with_dim(dim: usize) {
    let conn = Connection::open_in_memory().unwrap();
    nous_core::sqlite_vec::load(&conn).unwrap();

    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE test_vecs USING vec0(id TEXT PRIMARY KEY, embedding float[{dim}])"
    ))
    .unwrap();

    let vectors: Vec<(&str, Vec<f32>)> = vec![
        ("a", vec![1.0; dim]),
        ("b", vec![0.0; dim]),
        ("c", {
            let mut v = vec![0.5; dim];
            v[0] = 1.0;
            v
        }),
    ];

    for (id, emb) in &vectors {
        let blob: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
        conn.execute(
            "INSERT INTO test_vecs(id, embedding) VALUES (?, ?)",
            rusqlite::params![id, blob],
        )
        .unwrap();
    }

    let query_vec: Vec<u8> = vec![1.0f32; dim]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    let mut stmt = conn
        .prepare(
            "SELECT id, distance FROM test_vecs WHERE embedding MATCH ? ORDER BY distance LIMIT 5",
        )
        .unwrap();

    let results: Vec<(String, f64)> = stmt
        .query_map(rusqlite::params![query_vec], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].0, "a");
    assert!(
        (results[0].1 - 0.0).abs() < 1e-6,
        "first result should have distance ~0"
    );
    assert_eq!(results[1].0, "c");
    assert_eq!(results[2].0, "b");
}

#[test]
fn vec0_knn_round_trip() {
    vec0_knn_round_trip_with_dim(384);
}

#[test]
fn vec0_knn_round_trip_1024() {
    vec0_knn_round_trip_with_dim(1024);
}
