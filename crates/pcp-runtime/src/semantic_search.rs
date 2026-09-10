use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use infer_runtime_client::{
    Client, RetrievalEmbeddingRequest, RetrievalEmbeddingVector, RetrievalTextInput,
};
use pcp_client::PcpTenantApi;
use pcp_core::{BrowseIndexOrder, Projection, ReadPage, ReadPagesRequest, SearchHit};
use serde::{Deserialize, Serialize};
use tokio::{sync::Mutex, time::timeout};

use crate::{
    SemanticSearchConfig,
    query::{model_projection, projection_was_truncated, truncate_chars},
};

const CACHE_SCHEMA: u32 = 1;
const DOCUMENT_BATCH_SIZE: usize = 8;
// The Infer Runtime contract permits 64 inputs and 512 KiB in one request,
// but that is an import ceiling, not an interactive latency target. Keep a
// batch aligned with the bounded read window so a long historical Page cannot
// make the initial local index appear stalled.
const EMBEDDING_BATCH_SIZE: usize = 8;

/// Local, revision-keyed vector retrieval. It owns no Page content: all source
/// material is reread through the caller's PCP session before it is embedded.
#[derive(Clone)]
pub struct SemanticSearchProvider {
    client: Arc<Client>,
    cache_path: PathBuf,
    timeout: Duration,
    max_document_chars: usize,
    max_indexed_pages: usize,
    index: Arc<Mutex<SemanticIndex>>,
}

pub struct SemanticSearchResult {
    pub hits: Vec<SemanticSearchHit>,
    pub indexed_count: usize,
    pub embedded_count: usize,
    pub model_calls: usize,
}

pub struct SemanticSearchHit {
    pub hit: SearchHit,
    pub score: f32,
}

#[derive(Default, Deserialize, Serialize)]
struct SemanticIndex {
    schema: u32,
    #[serde(default)]
    entries: BTreeMap<String, CachedEmbedding>,
}

#[derive(Clone, Deserialize, Serialize)]
struct CachedEmbedding {
    page_id: String,
    namespace: String,
    embedding_space: String,
    values: Vec<f32>,
}

#[derive(Clone)]
struct SemanticDocument {
    hit: SearchHit,
    text: String,
}

impl SemanticSearchProvider {
    pub fn new(config: SemanticSearchConfig) -> Result<Self> {
        let index = load_index(&config.cache_path)?;
        Ok(Self {
            client: Arc::new(
                Client::builder()
                    .credential_file(config.credential_file)
                    .build()
                    .context("build Infer Runtime client for PCP semantic search")?,
            ),
            cache_path: config.cache_path,
            timeout: Duration::from_secs(config.timeout_seconds),
            max_document_chars: config.max_document_chars,
            max_indexed_pages: config.max_indexed_pages,
            index: Arc::new(Mutex::new(index)),
        })
    }

    pub async fn search(
        &self,
        client: &dyn PcpTenantApi,
        query: &str,
        scopes: &[String],
        limit: usize,
    ) -> Result<SemanticSearchResult> {
        let query_vector = self.embed_query(query).await?;
        let mut model_calls = 1usize;
        let query_space = validate_embedding(&query_vector, "query")?.to_owned();
        let (candidates, mut entries, documents) =
            self.prepare_documents(client, scopes, &query_space).await?;
        let embedded_count = documents.len();
        if !documents.is_empty() {
            let (new_embeddings, document_model_calls) =
                self.embed_documents(&documents, &query_space).await?;
            model_calls = model_calls.saturating_add(document_model_calls);
            let mut index = self.index.lock().await;
            index.entries.extend(new_embeddings.clone());
            index.schema = CACHE_SCHEMA;
            persist_index(&self.cache_path, &index)?;
            entries.extend(new_embeddings);
        }
        let indexed_count = entries.len();
        let mut hits = candidates
            .into_iter()
            .filter_map(|hit| {
                let entry = entries.get(&hit.revision_id)?;
                cosine_score(&query_vector.values, &entry.values)
                    .ok()
                    .map(|score| SemanticSearchHit {
                        hit: SearchHit {
                            matched_by: "semantic_vector".to_owned(),
                            matched_projection: "embedding".to_owned(),
                            ..hit
                        },
                        score,
                    })
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| right.score.total_cmp(&left.score));
        hits.truncate(limit);
        Ok(SemanticSearchResult {
            hits,
            indexed_count,
            embedded_count,
            model_calls,
        })
    }

    async fn prepare_documents(
        &self,
        client: &dyn PcpTenantApi,
        scopes: &[String],
        query_space: &str,
    ) -> Result<(
        Vec<SearchHit>,
        BTreeMap<String, CachedEmbedding>,
        Vec<SemanticDocument>,
    )> {
        let candidates = self.collect_candidates(client, scopes).await?;
        // Snapshot only authorized current revisions for this embedding space.
        // Concurrent queries may update the shared cache without changing this
        // request's vectors or causing a cached document to be embedded empty.
        let entries = {
            let index = self.index.lock().await;
            candidates
                .iter()
                .filter_map(|hit| {
                    let entry = index.entries.get(&hit.revision_id)?;
                    (entry.embedding_space == query_space
                        && entry.page_id == hit.page_id
                        && entry.namespace == hit.namespace)
                        .then(|| (hit.revision_id.clone(), entry.clone()))
                })
                .collect::<BTreeMap<_, _>>()
        };
        let missing = candidates
            .iter()
            .filter(|hit| !entries.contains_key(&hit.revision_id))
            .cloned()
            .collect::<Vec<_>>();
        let documents = self.read_documents(client, &missing).await?;
        Ok((candidates, entries, documents))
    }

    async fn embed_query(&self, query: &str) -> Result<RetrievalEmbeddingVector> {
        let response = timeout(
            self.timeout,
            self.client.embed_queries(&RetrievalEmbeddingRequest {
                model: "semantic.embed_query".to_owned(),
                inputs: vec![RetrievalTextInput {
                    id: "query".to_owned(),
                    text: query.to_owned(),
                    source_revision: "query:ephemeral".to_owned(),
                }],
                metadata: local_only_metadata(),
            }),
        )
        .await
        .context("PCP semantic query embedding timed out")??;
        ensure!(
            response.status == "completed",
            "Infer Runtime returned semantic query status {}",
            response.status
        );
        ensure!(
            response.data.len() == 1 && response.data[0].id == "query",
            "Infer Runtime returned an invalid semantic query embedding"
        );
        Ok(response
            .data
            .into_iter()
            .next()
            .expect("checked item")
            .embedding)
    }

    async fn embed_documents(
        &self,
        documents: &[SemanticDocument],
        query_space: &str,
    ) -> Result<(BTreeMap<String, CachedEmbedding>, usize)> {
        let mut entries = BTreeMap::new();
        let mut model_calls = 0usize;
        for batch in documents.chunks(EMBEDDING_BATCH_SIZE) {
            let request_inputs = batch
                .iter()
                .map(|document| RetrievalTextInput {
                    id: document.hit.revision_id.clone(),
                    text: document.text.clone(),
                    source_revision: document.hit.revision_id.clone(),
                })
                .collect::<Vec<_>>();
            let response = timeout(
                self.timeout,
                self.client.embed_documents(&RetrievalEmbeddingRequest {
                    model: "semantic.embed_documents".to_owned(),
                    inputs: request_inputs,
                    metadata: local_only_metadata(),
                }),
            )
            .await
            .context("PCP semantic document embedding timed out")??;
            model_calls = model_calls.saturating_add(1);
            ensure!(
                response.status == "completed",
                "Infer Runtime returned semantic document status {}",
                response.status
            );
            let documents_by_revision = batch
                .iter()
                .map(|document| (document.hit.revision_id.as_str(), document))
                .collect::<HashMap<_, _>>();
            ensure!(
                response.data.len() == documents_by_revision.len(),
                "Infer Runtime returned an incomplete semantic document batch"
            );
            for item in response.data {
                let document = documents_by_revision
                    .get(item.source_revision.as_str())
                    .context("Infer Runtime returned an unknown semantic document")?;
                ensure!(
                    item.id == document.hit.revision_id,
                    "Infer Runtime returned a mismatched semantic document identity"
                );
                let space = validate_embedding(&item.embedding, "document")?;
                ensure!(
                    space == query_space,
                    "Infer Runtime query and document embeddings have incompatible spaces"
                );
                entries.insert(
                    document.hit.revision_id.clone(),
                    CachedEmbedding {
                        page_id: document.hit.page_id.clone(),
                        namespace: document.hit.namespace.clone(),
                        embedding_space: space.to_owned(),
                        values: item.embedding.values,
                    },
                );
            }
        }
        Ok((entries, model_calls))
    }

    async fn collect_candidates(
        &self,
        client: &dyn PcpTenantApi,
        scopes: &[String],
    ) -> Result<Vec<SearchHit>> {
        let mut cursor = None;
        let mut hits = Vec::new();
        while hits.len() < self.max_indexed_pages {
            let page = client
                .browse_retrieval_pages(
                    scopes.to_vec(),
                    None,
                    BrowseIndexOrder::Recent,
                    50,
                    cursor,
                    client.capabilities().max_read_chars,
                )
                .await?;
            hits.extend(page.hits);
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        hits.truncate(self.max_indexed_pages);
        Ok(hits)
    }

    async fn read_documents(
        &self,
        client: &dyn PcpTenantApi,
        hits: &[SearchHit],
    ) -> Result<Vec<SemanticDocument>> {
        let mut documents = Vec::new();
        let max_read_chars = client.capabilities().max_read_chars;
        for chunk in hits.chunks(DOCUMENT_BATCH_SIZE) {
            let pages = client
                .read_pages(ReadPagesRequest {
                    page_ids: Vec::new(),
                    revision_ids: chunk.iter().map(|hit| hit.revision_id.clone()).collect(),
                    projections: vec![Projection::Payload, Projection::Summary],
                    max_chars: max_read_chars.min(
                        u32::try_from(self.max_document_chars * DOCUMENT_BATCH_SIZE)
                            .unwrap_or(u32::MAX),
                    ),
                })
                .await?;
            let pages = pages
                .into_iter()
                .map(|page| (page.revision.revision_id.clone(), page))
                .collect::<HashMap<_, _>>();
            for hit in chunk {
                let Some(page) = pages.get(&hit.revision_id) else {
                    continue;
                };
                let Some(text) = semantic_document_text(page, hit) else {
                    continue;
                };
                let (text, _) = truncate_chars(&text, self.max_document_chars);
                if !text.trim().is_empty() {
                    documents.push(SemanticDocument {
                        hit: hit.clone(),
                        text,
                    });
                }
            }
        }
        Ok(documents)
    }
}

fn semantic_document_text(page: &ReadPage, hit: &SearchHit) -> Option<String> {
    if let Some(summary) = &page.summary
        && summary.target_revision_id == hit.revision_id
        && !summary.content.trim().is_empty()
        && !projection_was_truncated(&summary.content)
    {
        return Some(summary.content.trim().to_owned());
    }
    let payload = page.revision.payload.as_ref()?;
    model_projection(&payload.media_type, &payload.content)
}

fn local_only_metadata() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("infer.placement".to_owned(), "local_only".to_owned()),
        ("infer.offline_required".to_owned(), "true".to_owned()),
        ("infer.fallback".to_owned(), "none".to_owned()),
    ])
}

fn validate_embedding<'a>(vector: &'a RetrievalEmbeddingVector, label: &str) -> Result<&'a str> {
    ensure!(
        vector.normalized
            && vector.distance_metric == "cosine"
            && vector.dimensions > 0
            && vector.values.len() == vector.dimensions
            && vector.values.iter().all(|value| value.is_finite())
            && !vector.space.trim().is_empty(),
        "Infer Runtime returned an invalid {label} embedding"
    );
    Ok(vector.space.as_str())
}

fn cosine_score(left: &[f32], right: &[f32]) -> Result<f32> {
    ensure!(
        left.len() == right.len(),
        "semantic embeddings have incompatible dimensions"
    );
    Ok(left.iter().zip(right).map(|(a, b)| a * b).sum())
}

fn load_index(path: &PathBuf) -> Result<SemanticIndex> {
    match fs::read(path) {
        Ok(bytes) => {
            let index: SemanticIndex = serde_json::from_slice(&bytes)
                .with_context(|| format!("decode PCP semantic cache {}", path.display()))?;
            Ok((index.schema == CACHE_SCHEMA)
                .then_some(index)
                .unwrap_or_default())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SemanticIndex::default()),
        Err(error) => {
            Err(error).with_context(|| format!("read PCP semantic cache {}", path.display()))
        }
    }
}

fn persist_index(path: &PathBuf, index: &SemanticIndex) -> Result<()> {
    let parent = path
        .parent()
        .context("PCP semantic cache path has no parent")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create PCP semantic cache directory {}", parent.display()))?;
    let bytes = serde_json::to_vec(index).context("encode PCP semantic cache")?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, bytes)
        .with_context(|| format!("write PCP semantic cache {}", temporary.display()))?;
    fs::rename(&temporary, path)
        .with_context(|| format!("replace PCP semantic cache {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cached_candidates_skip_body_reads_but_recheck_revisions_and_access() {
        use pcp_client::{EmbeddedPcpClient, PcpApi};
        use pcp_core::{AccessLogQuery, AccessPrincipal, AccessPrincipalType, AccessSession};
        let root =
            std::env::temp_dir().join(format!("pcp-semantic-cache-{}", uuid::Uuid::new_v4()));
        let store = Arc::new(
            pcp_sqlite::SqlitePcpStore::open(root.join("store.sqlite3"))
                .await
                .unwrap(),
        );
        let access = AccessSession::store_wide_full_control(
            AccessPrincipal {
                principal_id: "test:semantic".into(),
                principal_type: AccessPrincipalType::Service,
                display_name: None,
            },
            "test:semantic",
        );
        let client = EmbeddedPcpClient::new(store.clone(), access.clone());
        client
            .create_scope(pcp_core::CreateScopeRequest {
                namespace: "cache-test".into(),
                display_name: "Cache test".into(),
                description: None,
                parent_namespace: None,
            })
            .await
            .unwrap();
        let request = serde_json::from_value(serde_json::json!({
            "namespace":"cache-test", "lifecycleStatus":"active", "kind":"document", "mutability":"revisioned",
            "createdBy":{"actorId":"test", "actorType":"user"},
            "payload":{"mediaType":"text/plain", "content":"A semantic cache test document."}
        })).unwrap();
        let written = client.write_page(request).await.unwrap();
        let provider = SemanticSearchProvider::new(SemanticSearchConfig {
            credential_file: root.join("unused-credential.json"),
            cache_path: root.join("vectors.json"),
            timeout_seconds: 2,
            max_document_chars: 1000,
            max_indexed_pages: 100,
        })
        .unwrap();
        let reads = || async {
            client
                .query_access_log(AccessLogQuery {
                    operation: Some("read_pages".into()),
                    ..Default::default()
                })
                .await
                .unwrap()
                .total_events
        };
        let (_, cached, documents) = provider
            .prepare_documents(&client, &[], "space-a")
            .await
            .unwrap();
        assert!(cached.is_empty());
        assert_eq!(documents.len(), 1);
        assert!(!documents[0].text.is_empty());
        assert_eq!(reads().await, 1);
        provider.index.lock().await.entries.insert(
            written.revision_id.clone(),
            CachedEmbedding {
                page_id: written.page_id.clone(),
                namespace: "cache-test".into(),
                embedding_space: "space-a".into(),
                values: vec![1.0],
            },
        );
        let (hits, cached, documents) = provider
            .prepare_documents(&client, &[], "space-a")
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(cached.len(), 1);
        assert!(documents.is_empty());
        assert_eq!(reads().await, 1, "warm cache must avoid payload read_pages");
        // An embedding-space change needs source text again.
        assert_eq!(
            provider
                .prepare_documents(&client, &[], "space-b")
                .await
                .unwrap()
                .2
                .len(),
            1
        );
        assert_eq!(reads().await, 2);
        let revised = client
            .revise_page(
                serde_json::from_value(serde_json::json!({
                    "pageId":written.page_id, "expectedRevisionId":written.revision_id,
                    "lifecycleStatus":"active", "createdBy":{"actorId":"test", "actorType":"user"},
                    "payload":{"mediaType":"text/plain", "content":"Changed semantic document."}
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        let (hits, cached, documents) = provider
            .prepare_documents(&client, &[], "space-a")
            .await
            .unwrap();
        assert!(cached.is_empty());
        assert_eq!(documents.len(), 1);
        assert_eq!(hits[0].revision_id, revised.revision_id);
        assert_eq!(reads().await, 3);
        let restricted = EmbeddedPcpClient::new(
            store,
            AccessSession::new(access.principal, "no-access", Vec::new()),
        );
        let result = provider
            .prepare_documents(&restricted, &[], "space-a")
            .await;
        assert!(
            result.is_err() || result.unwrap().0.is_empty(),
            "cached IDs must not bypass current access"
        );
        assert_eq!(reads().await, 3);
        drop(client);
        drop(provider);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cosine_score_rejects_incompatible_vectors() {
        assert!(cosine_score(&[1.0], &[1.0, 0.0]).is_err());
    }

    #[test]
    fn cache_is_revision_keyed_and_contains_no_source_text() {
        let entry = CachedEmbedding {
            page_id: "pg_1".to_owned(),
            namespace: "project:example".to_owned(),
            embedding_space: "space".to_owned(),
            values: vec![0.5, 0.5],
        };
        let encoded = serde_json::to_string(&entry).expect("encode cache entry");
        assert!(!encoded.contains("source text"));
        assert!(encoded.contains("embedding_space"));
    }
}
