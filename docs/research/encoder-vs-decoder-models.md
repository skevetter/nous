# Encoder vs Decoder Embedding Models: Single or Dual?

**Date**: 2026-04-26
**Author**: Research Team
**Status**: Final

## Executive Summary

Offering two embedding models — a small encoder (BGE-small) and a larger decoder-derived model (Qwen3-Embedding) — mirrors exactly how OpenAI, Cohere, Voyage, and Jina structure their products: size tiers within one service, where each deployment picks one model. This is standard and sound, provided the system enforces that every vector index uses a single model. Nous should present BGE-small and Qwen3-Embedding as a quality-tier choice (fast/light vs accurate/heavy), not as models to combine in the same retrieval pipeline.

## Background: Encoder vs Decoder Embeddings

**Encoder models** (BERT, BGE, MiniLM) process all input tokens simultaneously with bidirectional attention. Every token attends to every other token, producing a fixed-size vector in a single forward pass. These models are fast, small (22M–335M parameters), and purpose-built for embedding.

**Decoder models** (GPT, LLaMA, Qwen) process tokens left-to-right with causal attention. Originally designed for text generation, recent work (NV-Embed, E5-Mistral, Qwen3-Embedding) fine-tunes them for embedding by pooling over the final hidden states. These models are larger (0.3B–7B+ parameters) and produce higher-quality embeddings at higher compute cost.

The "encoder vs decoder" label is increasingly a distinction about training origin rather than inference behavior. Qwen3-Embedding-0.6B, despite its decoder lineage, uses bidirectional attention for embedding extraction. What matters for deployment is model size, embedding dimension, and quality — not the architecture label.

## Industry Landscape

Every major embedding API provider offers size tiers within a single model family — none expose "encoder vs decoder" as a user-facing choice.

| Provider | Models | Dimensions | Tier Strategy |
|----------|--------|------------|---------------|
| OpenAI | text-embedding-3-{small, large} | 256–3072 | Two size tiers, one family |
| Cohere | embed-v4.0, embed-light-v3.0 | 256–1536 | Full and light variants |
| Jina AI | jina-embeddings-v5 (nano 239M, small 677M) | 32–1024+ (Matryoshka) | Size tiers per generation |
| Voyage AI | voyage-4-{large, standard, lite, nano} | 256–2048 | Four size tiers |
| Google | gemini-embedding-001 | Up to 3072 | Unified model replacing older variants |

Users pick small vs large based on quality-vs-cost needs. The underlying architecture (encoder, decoder, or hybrid) is an implementation detail the provider abstracts away. Nous offering BGE-small alongside Qwen3-Embedding follows this same pattern.

## Performance Comparison

### MTEB Benchmark Scores by Tier

**Small tier (<100M params)** — encoder-only territory:

| Model | Params | Dim | MTEB Avg (56 tasks) | Retrieval |
|-------|--------|-----|---------------------|-----------|
| BGE-small-en-v1.5 | 33.4M | 384 | 62.17 | 51.68 |
| all-MiniLM-L6-v2 | 22.7M | 384 | ~58–60 | — |
| e5-small-v2 | ~33M | 384 | 59.93 | 49.04 |

No decoder-derived models exist below 100M parameters. Encoder models own this tier.

**Mid tier (0.3B–1.5B params)** — decoder-derived models dominate:

| Model | Params | Dim | MTEB English | Architecture |
|-------|--------|-----|-------------|--------------|
| Qwen3-Embedding-0.6B | 600M | 1024 | 70.70 | Decoder-derived |
| multilingual-e5-large-instruct | 600M | 1024 | 65.53 | Encoder (T5) |
| BGE-large-en-v1.5 | 335M | 1024 | 64.23 | Encoder (BERT) |

**Top tier (>1B params)** — decoder-based models lead the leaderboard: NV-Embed (7B), E5-Mistral-7B, SFR-Embedding.

### Resource and Cost Trade-offs

| | BGE-small-en-v1.5 | Qwen3-Embedding-0.6B |
|---|---|---|
| Parameters | 33.4M | 600M (18x) |
| Embedding dimension | 384 | 1024 |
| Memory (fp16) | ~67 MB | ~1.2 GB (18x) |
| Relative latency | ~1.5x baseline | ~25–30x baseline |
| Storage per vector | 1.5 KB | 4 KB (2.7x) |
| Storage per 1M docs | 1.5 GB | 4 GB |
| MTEB English | 62.17 | 70.70 (+8.5 pts) |
| Max tokens | 512 | 32,768 |

The 8.5-point quality gap is meaningful across diverse NLP tasks (retrieval, classification, clustering, STS). The cost is 18x more compute and 2.7x more storage.

## Best Practices: Single vs Multiple Models

**Industry consensus: one embedding model per vector index.**

Three constraints make this non-negotiable:

1. **Dimensional compatibility** — BGE-small produces 384-dimensional vectors; Qwen3-Embedding produces 1024-dimensional vectors. These cannot coexist in the same index. Cosine similarity requires identical vector spaces.
2. **Model consistency** — Query embeddings and document embeddings must come from the same model. A query embedded with BGE-small cannot meaningfully search a Qwen3-Embedding index.
3. **Semantic space alignment** — Even two models with the same dimensionality encode meaning differently. Mixing models degrades retrieval quality.

### When multiple models ARE appropriate

| Use Case | Example | Standard? |
|----------|---------|-----------|
| Quality tiers | User picks small (fast) vs large (accurate) per deployment | Yes — this is the OpenAI/Cohere/Voyage pattern |
| Model migration | Re-embed corpus when upgrading from v2 to v3 | Yes — temporary dual state during transition |
| Different modalities | Text embeddings (BGE) + image embeddings (CLIP) | Yes — different data types, different indices |
| A/B testing | Parallel indices to evaluate a candidate model | Yes — temporary evaluation |
| Same corpus, same task, two models simultaneously | Query with both models and merge results | No — not standard, adds complexity without clear benefit |

### Embedding + re-ranker (common confusion)

Production RAG systems often use two models: an embedding model for retrieval and a cross-encoder re-ranker for refinement. The re-ranker is NOT a second embedding model — it scores (query, document) pairs directly and does not produce storable vectors. This two-stage pattern (bi-encoder retrieval → cross-encoder re-ranking) is standard and distinct from running two embedding models.

## Recommendation for Nous

Nous currently supports MiniLM-L6-v2 (384d) and BGE-small (384d) via ONNX, with plans to add Qwen3-Embedding-0.6B (1024d) via KV-cache and BGE-base/Nomic at 768d.

**Supporting multiple models is the right approach** — with three guardrails:

1. **One model per index.** When a user creates a knowledge base, they pick a model. All documents and queries in that index must use that model. Nous already tracks `model_id` on the `EmbeddingBackend` trait (`crates/nous-core/src/embed.rs`) and stores it per chunk in the database — query-time enforcement should be added to reject cross-model queries.

2. **Present as quality tiers, not architecture choices.** Users should see "light (fast, 384d)" vs "standard (accurate, 1024d)" — not "encoder" vs "decoder." The architecture is an implementation detail.

3. **Default to the small model.** BGE-small at 62.17 MTEB with 67 MB memory is the right default for most local-first deployments. Qwen3-Embedding is the upgrade path when users need higher retrieval quality and have the compute budget (1.2 GB memory, ~20x latency increase).

**Planned model roadmap fits this pattern:**

| Tier | Model | Dim | Use Case |
|------|-------|-----|----------|
| Light | MiniLM-L6-v2 / BGE-small | 384 | Edge, CI, resource-constrained |
| Standard | BGE-base / Nomic | 768 | Balanced quality/cost |
| Premium | Qwen3-Embedding-0.6B | 1024 | Maximum retrieval quality |

Note: the current vec0 virtual table hardcodes `float[384]` dimensions. Supporting 768d and 1024d tiers requires schema work (per-model virtual tables or dynamic dimension configuration) as part of the KV-cache design implementation.

This tiered approach mirrors the OpenAI small/large, Voyage nano/lite/standard/large, and Cohere full/light tier structures. It is standard, well-understood by users, and technically sound.

## Sources

- [MTEB Leaderboard](https://huggingface.co/spaces/mteb/leaderboard)
- [BGE-small-en-v1.5 model card](https://huggingface.co/BAAI/bge-small-en-v1.5)
- [Qwen3-Embedding-0.6B model card](https://huggingface.co/Qwen/Qwen3-Embedding-0.6B)
- [NV-Embed: "Improved Techniques for Training LLMs as Generalist Embedding Models"](https://arxiv.org/abs/2405.17428)
- [E5-Mistral: "Improving Text Embeddings with Large Language Models"](https://arxiv.org/abs/2401.00368)
- [Sentence-Transformers pretrained models](https://sbert.net)
- [Cohere embed models](https://docs.cohere.com)
- [Jina AI embeddings](https://jina.ai/embeddings)
- [Voyage AI embeddings](https://docs.voyageai.com)
- [Google Vertex AI embedding models](https://cloud.google.com/vertex-ai)
- [LlamaIndex RAG embedding/reranker evaluation](https://llamaindex.ai/blog)
