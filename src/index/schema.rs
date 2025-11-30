use crate::session::{SearchResult, Session, SessionSource};
use anyhow::{Context, Result};
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, BoostQuery, Occur, PhraseQuery, Query, QueryParser};
use tantivy::schema::*;
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy};

/// Wrapper around Tantivy index for session search
pub struct SessionIndex {
    index: Index,
    reader: IndexReader,
    #[allow(dead_code)]
    schema: Schema,
    // Field handles
    session_id: Field,
    source: Field,
    file_path: Field,
    cwd: Field,
    git_branch: Field,
    timestamp: Field,
    content: Field,
    message_index: Field,
}

impl SessionIndex {
    /// Open existing index or create a new one
    pub fn open_or_create(index_path: &Path) -> Result<Self> {
        std::fs::create_dir_all(index_path)?;

        let schema = Self::build_schema();

        let index = if index_path.join("meta.json").exists() {
            Index::open_in_dir(index_path).context("Failed to open existing index")?
        } else {
            Index::create_in_dir(index_path, schema.clone())
                .context("Failed to create new index")?
        };

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .context("Failed to create index reader")?;

        Ok(Self {
            index,
            reader,
            session_id: schema.get_field("session_id").unwrap(),
            source: schema.get_field("source").unwrap(),
            file_path: schema.get_field("file_path").unwrap(),
            cwd: schema.get_field("cwd").unwrap(),
            git_branch: schema.get_field("git_branch").unwrap(),
            timestamp: schema.get_field("timestamp").unwrap(),
            content: schema.get_field("content").unwrap(),
            message_index: schema.get_field("message_index").unwrap(),
            schema,
        })
    }

    fn build_schema() -> Schema {
        let mut builder = Schema::builder();

        // Stored metadata fields
        builder.add_text_field("session_id", STRING | STORED);
        builder.add_text_field("source", STRING | STORED);
        builder.add_text_field("file_path", STRING | STORED);
        builder.add_text_field("cwd", STRING | STORED);
        builder.add_text_field("git_branch", STRING | STORED);

        // Timestamp for recency sorting (stored as i64 unix timestamp)
        builder.add_i64_field("timestamp", INDEXED | STORED | FAST);

        // Message index within the session (for match-recency)
        builder.add_u64_field("message_index", STORED);

        // Searchable content field
        builder.add_text_field("content", TEXT | STORED);

        builder.build()
    }

    /// Get a writer for indexing operations
    pub fn writer(&self) -> Result<IndexWriter> {
        self.index
            .writer(50_000_000) // 50MB heap
            .context("Failed to create index writer")
    }

    /// Index a single session (all its messages)
    pub fn index_session(&self, writer: &mut IndexWriter, session: &Session) -> Result<()> {
        let timestamp_secs = session.timestamp.timestamp();

        // Index each message separately for match-recency ranking
        for (idx, message) in session.messages.iter().enumerate() {
            let doc = doc!(
                self.session_id => session.id.clone(),
                self.source => session.source.as_str(),
                self.file_path => session.file_path.to_string_lossy().to_string(),
                self.cwd => session.cwd.clone(),
                self.git_branch => session.git_branch.clone().unwrap_or_default(),
                self.timestamp => timestamp_secs,
                self.message_index => idx as u64,
                self.content => message.content.clone(),
            );
            writer.add_document(doc)?;
        }

        Ok(())
    }

    /// Delete all documents for a session (by file path)
    pub fn delete_session(&self, writer: &mut IndexWriter, file_path: &Path) {
        let term = tantivy::Term::from_field_text(
            self.file_path,
            &file_path.to_string_lossy(),
        );
        writer.delete_term(term);
    }

    /// Reload the reader to see recent changes
    pub fn reload(&self) -> Result<()> {
        self.reader.reload().context("Failed to reload reader")
    }

    /// Search for sessions matching the query
    /// Returns results grouped by session, ranked by match-recency
    pub fn search(&self, query_str: &str, limit: usize) -> Result<Vec<SearchResult>> {
        if query_str.trim().is_empty() {
            return Ok(Vec::new());
        }

        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.content]);

        let base_query = query_parser
            .parse_query(query_str)
            .context("Failed to parse query")?;

        // Boost exact phrase matches for multi-word queries
        let words: Vec<&str> = query_str.split_whitespace().collect();
        let query: Box<dyn Query> = if words.len() > 1 {
            // Create phrase query for exact match boost
            let terms: Vec<tantivy::Term> = words
                .iter()
                .map(|w| tantivy::Term::from_field_text(self.content, &w.to_lowercase()))
                .collect();
            let phrase_query = PhraseQuery::new(terms);
            let boosted_phrase = BoostQuery::new(Box::new(phrase_query), 5.0);

            // Combine: phrase (boosted) OR terms
            Box::new(BooleanQuery::new(vec![
                (Occur::Should, Box::new(boosted_phrase) as Box<dyn Query>),
                (Occur::Should, base_query),
            ]))
        } else {
            base_query
        };

        // Get more results than limit to group by session
        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit * 10))?;

        // Group by session, keeping track of the highest-scoring message per session
        let mut session_results: std::collections::HashMap<String, (f32, SearchResult)> =
            std::collections::HashMap::new();

        for (score, doc_addr) in top_docs {
            let doc: tantivy::TantivyDocument = searcher.doc(doc_addr)?;

            let session_id = doc
                .get_first(self.session_id)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let source_str = doc
                .get_first(self.source)
                .and_then(|v| v.as_str())
                .unwrap_or("claude");

            let source = SessionSource::parse(source_str).unwrap_or(SessionSource::ClaudeCode);

            let file_path = doc
                .get_first(self.file_path)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let cwd = doc
                .get_first(self.cwd)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let git_branch = doc
                .get_first(self.git_branch)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty());

            let timestamp_secs = doc
                .get_first(self.timestamp)
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let message_index = doc
                .get_first(self.message_index)
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;

            let content = doc
                .get_first(self.content)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Create snippet around the match (store more, truncate in UI)
            let snippet = create_snippet(&content, query_str, 200);
            let match_spans = find_match_spans(&snippet, query_str);

            let result = SearchResult {
                session: Session {
                    id: session_id.clone(),
                    source,
                    file_path: std::path::PathBuf::from(&file_path),
                    cwd,
                    git_branch,
                    timestamp: chrono::DateTime::from_timestamp(timestamp_secs, 0)
                        .unwrap_or_default(),
                    messages: Vec::new(), // We don't load all messages for search results
                },
                score,
                matched_message_index: message_index,
                snippet,
                match_spans,
            };

            // Keep the highest-scoring result for each session
            // But prefer more recent message indices (higher = more recent)
            session_results
                .entry(session_id)
                .and_modify(|(existing_score, existing_result)| {
                    // Prefer higher message index (more recent) if scores are similar
                    let recency_bonus = (message_index as f32) * 0.01;
                    if score + recency_bonus > *existing_score {
                        *existing_score = score + recency_bonus;
                        *existing_result = result.clone();
                    }
                })
                .or_insert((score, result));
        }

        // Sort by combined relevance + recency score
        // Recency boost: exponential decay with ~7 day half-life
        let now = chrono::Utc::now().timestamp() as f64;
        let half_life_secs = 7.0 * 24.0 * 3600.0; // 7 days

        let mut results: Vec<_> = session_results.into_values().map(|(_, r)| r).collect();
        results.sort_by(|a, b| {
            let age_a = (now - a.session.timestamp.timestamp() as f64).max(0.0);
            let age_b = (now - b.session.timestamp.timestamp() as f64).max(0.0);

            // Exponential decay: recent sessions get boost up to 2x
            let recency_a = 1.0 + (-age_a / half_life_secs).exp();
            let recency_b = 1.0 + (-age_b / half_life_secs).exp();

            let final_a = (a.score as f64) * recency_a;
            let final_b = (b.score as f64) * recency_b;

            final_b.partial_cmp(&final_a).unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);

        Ok(results)
    }

    /// Get recent sessions sorted by timestamp (most recent first)
    pub fn recent(&self, limit: usize) -> Result<Vec<SearchResult>> {
        use tantivy::collector::TopDocs;
        use tantivy::query::AllQuery;

        let searcher = self.reader.searcher();

        // Get all docs sorted by timestamp descending
        // Fetch many more docs since each session has multiple messages indexed
        let top_docs = searcher.search(
            &AllQuery,
            &TopDocs::with_limit(limit * 100).order_by_fast_field::<i64>("timestamp", tantivy::Order::Desc),
        )?;

        // Group by session, keeping only the most recent per session
        let mut session_results: std::collections::HashMap<String, SearchResult> =
            std::collections::HashMap::new();

        for (_score, doc_addr) in top_docs {
            let doc: tantivy::TantivyDocument = searcher.doc(doc_addr)?;

            let session_id = doc
                .get_first(self.session_id)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Skip if we already have this session
            if session_results.contains_key(&session_id) {
                continue;
            }

            let source_str = doc
                .get_first(self.source)
                .and_then(|v| v.as_str())
                .unwrap_or("claude");

            let source = SessionSource::parse(source_str).unwrap_or(SessionSource::ClaudeCode);

            let file_path = doc
                .get_first(self.file_path)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let cwd = doc
                .get_first(self.cwd)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let git_branch = doc
                .get_first(self.git_branch)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty());

            let timestamp_secs = doc
                .get_first(self.timestamp)
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let content = doc
                .get_first(self.content)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Use first part of content as snippet
            let snippet: String = content.chars().take(200).collect();
            let snippet = snippet.replace('\n', " ");

            let result = SearchResult {
                session: Session {
                    id: session_id.clone(),
                    source,
                    file_path: std::path::PathBuf::from(&file_path),
                    cwd,
                    git_branch,
                    timestamp: chrono::DateTime::from_timestamp(timestamp_secs, 0)
                        .unwrap_or_default(),
                    messages: Vec::new(),
                },
                score: 0.0,
                matched_message_index: 0,
                snippet,
                match_spans: Vec::new(),
            };

            session_results.insert(session_id, result);

            if session_results.len() >= limit {
                break;
            }
        }

        // Sort by timestamp descending
        let mut results: Vec<_> = session_results.into_values().collect();
        results.sort_by(|a, b| b.session.timestamp.cmp(&a.session.timestamp));
        results.truncate(limit);

        Ok(results)
    }
}

/// Create a snippet around the first match
fn create_snippet(content: &str, query: &str, max_len: usize) -> String {
    // Work with chars to avoid Unicode boundary issues
    let chars: Vec<char> = content.chars().collect();
    let lower_content: String = chars.iter().collect::<String>().to_lowercase();
    let lower_query = query.to_lowercase();

    // Find first match (in char indices)
    if let Some(byte_pos) = lower_content.find(&lower_query) {
        // Convert byte position to char position
        let char_pos = lower_content[..byte_pos].chars().count();
        let query_char_len = lower_query.chars().count();

        let start_char = char_pos.saturating_sub(max_len / 2);
        let end_char = (char_pos + query_char_len + max_len / 2).min(chars.len());

        let mut snippet: String = chars[start_char..end_char].iter().collect();

        // Add ellipsis if truncated
        if start_char > 0 {
            snippet = format!("...{}", snippet.trim_start());
        }
        if end_char < chars.len() {
            snippet = format!("{}...", snippet.trim_end());
        }

        // Clean up newlines
        snippet.replace('\n', " ")
    } else {
        // No match found, just truncate
        let truncated: String = chars.iter().take(max_len).collect();
        if chars.len() > max_len {
            format!("{}...", truncated.trim())
        } else {
            truncated
        }
    }
}

/// Find byte ranges of query matches within a snippet (for highlighting)
fn find_match_spans(snippet: &str, query: &str) -> Vec<(usize, usize)> {
    let lower_snippet = snippet.to_lowercase();
    let lower_query = query.to_lowercase();

    let mut spans = Vec::new();
    let mut start = 0;

    while let Some(pos) = lower_snippet[start..].find(&lower_query) {
        let abs_pos = start + pos;
        spans.push((abs_pos, abs_pos + query.len()));
        start = abs_pos + query.len();
    }

    spans
}
