use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct FeedParams {
    pub(super) stream: Option<String>,
    pub(super) source_id: Option<String>,
    pub(super) days: Option<i32>,
    pub(super) include_dismissed: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct QualityParams {
    pub(super) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct QualityRefreshBody {
    pub(super) days: Option<i32>,
}

#[derive(Debug, Serialize)]
pub(super) struct RelevanceOut {
    pub(super) profile_key: String,
    pub(super) profile_label: String,
    pub(super) score: f64,
    pub(super) rationale: String,
    pub(super) mode: String,
    pub(super) profile_revision: String,
}

impl From<RelevanceMatch> for RelevanceOut {
    fn from(relevance: RelevanceMatch) -> Self {
        Self {
            profile_key: relevance.profile_key,
            profile_label: relevance.profile_label,
            score: relevance.score,
            rationale: relevance.rationale,
            mode: relevance.mode,
            profile_revision: relevance.profile_revision,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct EvaluationFactorOut {
    pub(super) key: String,
    pub(super) label: String,
    pub(super) score: f64,
    pub(super) weight: f64,
    pub(super) rationale: String,
    pub(super) context: Option<evaluation::EvaluationFactorContext>,
}

impl From<EvaluationFactor> for EvaluationFactorOut {
    fn from(factor: EvaluationFactor) -> Self {
        Self {
            key: factor.key,
            label: factor.label,
            score: factor.score,
            weight: factor.weight,
            rationale: factor.rationale,
            context: factor.context,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct EvaluationOut {
    pub(super) overall_score: f64,
    pub(super) explanation: String,
    pub(super) mode: String,
    pub(super) item_revision: String,
    pub(super) context_revision: String,
    pub(super) evaluator_revision: String,
    pub(super) evaluated_at: String,
    pub(super) factors: Vec<EvaluationFactorOut>,
}

impl From<FeedEvaluation> for EvaluationOut {
    fn from(evaluation: FeedEvaluation) -> Self {
        Self {
            overall_score: evaluation.overall_score,
            explanation: evaluation.explanation,
            mode: evaluation.mode,
            item_revision: evaluation.item_revision,
            context_revision: evaluation.context_revision,
            evaluator_revision: evaluation.evaluator_revision,
            evaluated_at: evaluation.evaluated_at,
            factors: evaluation
                .factors
                .into_iter()
                .map(EvaluationFactorOut::from)
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct OriginOut {
    pub(super) source_id: String,
    pub(super) source_ref: String,
    pub(super) label: Option<String>,
}

impl From<FeedOrigin> for OriginOut {
    fn from(origin: FeedOrigin) -> Self {
        Self {
            source_id: origin.source_id,
            source_ref: origin.source_ref,
            label: origin.label,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct StageProvenanceOut {
    pub(super) stage: String,
    pub(super) tier: String,
    pub(super) revision: String,
    pub(super) completed_at: String,
}

impl From<StageProvenance> for StageProvenanceOut {
    fn from(value: StageProvenance) -> Self {
        Self {
            stage: value.stage,
            tier: value.tier,
            revision: value.revision,
            completed_at: value.completed_at,
        }
    }
}

/// List payload omits the transcript and carries only the strongest TELOS
/// match. The reader endpoint returns every stored match.
#[derive(Debug, Serialize)]
pub(super) struct FeedListItem {
    pub(super) id: String,
    pub(super) stream: String,
    pub(super) kind: String,
    pub(super) title: Option<String>,
    pub(super) url: String,
    pub(super) author: Option<String>,
    pub(super) summary: Option<String>,
    /// The opening of this item's digest, for a card that has no summary of its own.
    ///
    /// A SEPARATE field rather than a fallback written into `summary`, because the two have
    /// different producers and different lengths, and collapsing them would make the card claim a
    /// summary that no summarization pass produced. The client chooses; the server does not
    /// pretend.
    ///
    /// Why it is needed: the enrichment drain (`media::summarize_pending`) runs on the light rung
    /// only and has no cloud door -- `digest::over_window` is the one that opens one. So an item
    /// past the 4,096-token window gets a digest and never a summary, and 66 items sat with a
    /// perfectly good digest behind an empty card (measured 2026-08-30). This spends nothing: the
    /// digest already exists.
    pub(super) digest_preview: Option<String>,
    pub(super) day: String,
    pub(super) created_at: String,
    pub(super) status: String,
    pub(super) relevance: Option<RelevanceOut>,
    pub(super) evaluation: Option<EvaluationOut>,
}

impl FeedListItem {
    /// How much of a digest is a card preview. Long enough to be worth reading, short enough
    /// that the card stays a card; the digest itself is one fetch away on the item.
    const DIGEST_PREVIEW_CHARS: usize = 320;

    pub(super) fn from_store(
        item: FeedItem,
        relevance: Option<RelevanceMatch>,
        evaluation: Option<FeedEvaluation>,
        digest: Option<String>,
    ) -> Self {
        Self {
            id: item.id,
            stream: item.stream,
            kind: item.kind,
            title: item.title,
            url: item.url,
            author: item.author,
            summary: item.summary,
            // Only when there is nothing better. An item with its own summary keeps it: the
            // digest is the longer, later artefact, and preferring it would replace a card's
            // preview with a heading tree the moment one appeared.
            digest_preview: digest.filter(|text| !text.trim().is_empty()).map(|text| {
                text.trim()
                    .chars()
                    .take(Self::DIGEST_PREVIEW_CHARS)
                    .collect()
            }),
            day: item.day,
            created_at: item.created_at,
            status: item.status,
            relevance: relevance.map(RelevanceOut::from),
            evaluation: evaluation.map(EvaluationOut::from),
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct FeedFullItem {
    pub(super) id: String,
    pub(super) stream: String,
    pub(super) kind: String,
    pub(super) title: Option<String>,
    pub(super) url: String,
    pub(super) author: Option<String>,
    pub(super) summary: Option<String>,
    pub(super) transcript: Option<String>,
    pub(super) day: String,
    pub(super) created_at: String,
    pub(super) status: String,
    pub(super) content_status: String,
    /// Which client handed this content over; null when the server fetched it.
    pub(super) captured_via: Option<String>,
    pub(super) relevance: Vec<RelevanceOut>,
    pub(super) evaluation: Option<EvaluationOut>,
    pub(super) processing: Vec<StageProvenanceOut>,
    pub(super) origins: Vec<OriginOut>,
}

impl FeedFullItem {
    pub(super) fn from_store(
        item: FeedItem,
        relevance: Vec<RelevanceMatch>,
        evaluation: Option<FeedEvaluation>,
        processing: Vec<StageProvenance>,
        origins: Vec<FeedOrigin>,
    ) -> Self {
        Self {
            id: item.id,
            stream: item.stream,
            kind: item.kind,
            title: item.title,
            url: item.url,
            author: item.author,
            summary: item.summary,
            transcript: item.transcript,
            day: item.day,
            created_at: item.created_at,
            status: item.status,
            content_status: item.content_status,
            captured_via: item.captured_via,
            relevance: relevance.into_iter().map(RelevanceOut::from).collect(),
            evaluation: evaluation.map(EvaluationOut::from),
            processing: processing
                .into_iter()
                .map(StageProvenanceOut::from)
                .collect(),
            origins: origins.into_iter().map(OriginOut::from).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct TriageOut {
    pub(super) id: String,
    pub(super) from_addr: Option<String>,
    pub(super) subject: Option<String>,
    pub(super) snippet: Option<String>,
    pub(super) internal_date: Option<String>,
    pub(super) stream: String,
    pub(super) rationale: String,
    pub(super) classification_method: String,
    pub(super) classification_version: String,
    pub(super) data_class: String,
    pub(super) data_class_rationale: String,
    pub(super) data_classification_method: String,
    pub(super) data_classification_version: String,
    pub(super) status: String,
    pub(super) gmail_action: Option<String>,
    pub(super) gmail_action_at: Option<String>,
    pub(super) purge_after: Option<String>,
    pub(super) gmail_location: Option<String>,
    pub(super) gmail_observed_at: Option<String>,
    pub(super) gmail_sync_status: Option<String>,
    pub(super) gmail_sync_action: Option<String>,
    pub(super) gmail_sync_error: Option<String>,
    /// The doctrine's one state label. Rendered as a badge rather than folded into
    /// `status`: status is what Axon decided about a proposal, waiting is what the
    /// operator decided about the conversation, and collapsing them would make
    /// "I replied and I'm blocked" indistinguishable from "Axon dismissed it".
    pub(super) waiting: bool,
    pub(super) waiting_since: Option<String>,
    pub(super) first_seen: String,
    pub(super) last_seen: String,
    pub(super) relevance: Vec<RelevanceOut>,
}

impl TriageOut {
    pub(super) fn from_store(item: TriageItem, relevance: Vec<RelevanceMatch>) -> Self {
        Self {
            id: item.id,
            from_addr: item.from_addr,
            subject: item.subject,
            snippet: item.snippet,
            internal_date: item.internal_date_text,
            stream: item.stream,
            rationale: item.rationale,
            classification_method: item.classification_method,
            classification_version: item.classification_version,
            data_class: item.data_class,
            data_class_rationale: item.data_class_rationale,
            data_classification_method: item.data_classification_method,
            data_classification_version: item.data_classification_version,
            status: item.status,
            gmail_action: item.gmail_action,
            gmail_action_at: item.gmail_action_at,
            purge_after: item.purge_after,
            gmail_location: item.gmail_location,
            gmail_observed_at: item.gmail_observed_at,
            gmail_sync_status: item.gmail_sync_status,
            gmail_sync_action: item.gmail_sync_action,
            gmail_sync_error: item.gmail_sync_error,
            waiting: item.waiting,
            waiting_since: item.waiting_since,
            first_seen: item.first_seen,
            last_seen: item.last_seen,
            relevance: relevance.into_iter().map(RelevanceOut::from).collect(),
        }
    }
}

/// Source-specific fields attached to the canonical content reader contract.
/// They extend the content item without forcing Gmail workflow state into
/// every other Feed source.
#[derive(Debug, Serialize)]
pub(super) struct MailContentExtensionOut {
    pub(super) category: String,
    pub(super) rationale: String,
    pub(super) classification_method: String,
    pub(super) classification_version: String,
    pub(super) gmail_action: Option<String>,
    pub(super) gmail_action_at: Option<String>,
    pub(super) purge_after: Option<String>,
    pub(super) gmail_location: Option<String>,
    pub(super) gmail_observed_at: Option<String>,
    pub(super) gmail_sync_status: Option<String>,
    pub(super) gmail_sync_action: Option<String>,
    pub(super) gmail_sync_error: Option<String>,
}

/// One reader shape for every kind of observed content. Source adapters own
/// collection and actions; the dashboard owns one renderer for this contract.
#[derive(Debug, Serialize)]
pub(super) struct ContentItemOut {
    pub(super) schema_version: &'static str,
    pub(super) source: &'static str,
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) title: Option<String>,
    pub(super) url: String,
    pub(super) author: Option<String>,
    pub(super) summary: Option<String>,
    pub(super) content: Option<String>,
    pub(super) content_label: String,
    pub(super) day: String,
    pub(super) created_at: String,
    pub(super) status: String,
    pub(super) content_status: String,
    pub(super) data_class: DataClass,
    pub(super) processing_policy: content_item::ProcessingPolicy,
    pub(super) cloud_processing: CloudDerivativeState,
    pub(super) relevance: Vec<RelevanceOut>,
    pub(super) evaluation: Option<EvaluationOut>,
    pub(super) processing: Vec<StageProvenanceOut>,
    pub(super) origins: Vec<OriginOut>,
    pub(super) digest: Option<content_item::Digest>,
    pub(super) mail: Option<MailContentExtensionOut>,
}

impl ContentItemOut {
    pub(super) fn from_feed(
        item: FeedItem,
        relevance: Vec<RelevanceMatch>,
        evaluation: Option<FeedEvaluation>,
        processing: Vec<StageProvenance>,
        origins: Vec<FeedOrigin>,
    ) -> Self {
        // The stored class, not a literal. This line used to call a constructor
        // that stamped c0 on every feed item on its way out of the store -- and
        // c0 is precisely the value `cloud_derivative::tier_allows` admits for a
        // verbatim document, so the entire feed was cloud-eligible verbatim
        // without anyone having decided that about a single item. The row
        // carries the answer now, and it defaults to c1.
        let classification = DataClass::stored(
            &item.data_class,
            &item.data_class_rationale,
            &item.data_classification_method,
            &item.data_classification_version,
        );
        let processing_policy = content_item::processing_policy(&classification.value);
        let content_label = match item.kind.as_str() {
            "github" => "README",
            "arxiv" => "Abstract",
            "youtube" | "podcast" | "instagram" => "Transcript",
            _ => "Article content",
        };
        Self {
            schema_version: "content-item-v2",
            source: "feed",
            id: item.id,
            kind: item.kind,
            title: item.title,
            url: item.url,
            author: item.author,
            summary: item.summary,
            content: item.transcript,
            content_label: content_label.into(),
            day: item.day,
            created_at: item.created_at,
            status: item.status,
            content_status: item.content_status,
            data_class: classification,
            processing_policy,
            cloud_processing: CloudDerivativeState::not_prepared(),
            relevance: relevance.into_iter().map(RelevanceOut::from).collect(),
            evaluation: evaluation.map(EvaluationOut::from),
            processing: processing
                .into_iter()
                .map(StageProvenanceOut::from)
                .collect(),
            origins: origins.into_iter().map(OriginOut::from).collect(),
            // Filled by `attach_digest` -- a projection cannot query.
            digest: None,
            mail: None,
        }
    }

    pub(super) fn from_mail(item: TriageItem, relevance: Vec<RelevanceMatch>) -> Self {
        let created_at = item
            .internal_date_text
            .clone()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| item.first_seen.clone());
        let day = created_at.get(..10).unwrap_or_default().to_string();
        let content_status = if item
            .snippet
            .as_deref()
            .is_some_and(|snippet| !snippet.trim().is_empty())
        {
            "thin"
        } else {
            "none"
        };
        let classification = DataClass::new(
            item.data_class.clone(),
            item.data_class_rationale.clone(),
            item.data_classification_method.clone(),
            item.data_classification_version.clone(),
        );
        let processing_policy = content_item::processing_policy(&classification.value);
        Self {
            schema_version: "content-item-v2",
            source: "mail",
            url: format!("https://mail.google.com/mail/u/0/#all/{}", item.id),
            id: item.id,
            kind: "mail".into(),
            title: item.subject,
            author: item.from_addr,
            summary: None,
            content: item.snippet,
            content_label: "Message preview".into(),
            day,
            created_at,
            status: item.status,
            content_status: content_status.into(),
            data_class: classification,
            processing_policy,
            cloud_processing: CloudDerivativeState::not_prepared(),
            relevance: relevance.into_iter().map(RelevanceOut::from).collect(),
            evaluation: None,
            processing: Vec::new(),
            origins: Vec::new(),
            digest: None,
            mail: Some(MailContentExtensionOut {
                category: item.stream,
                rationale: item.rationale,
                classification_method: item.classification_method,
                classification_version: item.classification_version,
                gmail_action: item.gmail_action,
                gmail_action_at: item.gmail_action_at,
                purge_after: item.purge_after,
                gmail_location: item.gmail_location,
                gmail_observed_at: item.gmail_observed_at,
                gmail_sync_status: item.gmail_sync_status,
                gmail_sync_action: item.gmail_sync_action,
                gmail_sync_error: item.gmail_sync_error,
            }),
        }
    }

    pub(super) fn cloud_input(&self) -> CloudDocumentInput {
        CloudDocumentInput {
            source: self.source.into(),
            id: self.id.clone(),
            title: self.title.clone(),
            author: self.author.clone(),
            summary: self.summary.clone(),
            content: self.content.clone(),
            data_class: self.data_class.value.clone(),
        }
    }

    /// Read the stored digest, if one exists.
    ///
    /// Reads only. A GET that quietly runs a local model turns opening an item
    /// into a two-minute wait and a load nobody asked for; generating is always
    /// an explicit press or the bounded pass.
    pub(super) fn attach_digest(
        mut self,
        store: &Store,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        self.digest = store
            .content_digest(self.source, &self.id)?
            .as_ref()
            .map(digest::to_contract);
        Ok(self)
    }

    pub(super) fn attach_cloud_state(
        mut self,
        store: &Store,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // A c2 or c3 item has no derivative and never could have had one, so
        // there is no state to read and the field keeps its `not_prepared`
        // value. The
        // reader sees what it saw before; what changed is that the document is
        // no longer assembled and hashed on the way to discovering that nobody
        // may use it.
        let Ok(preview) = cloud_derivative::prepare(&self.cloud_input()) else {
            return Ok(self);
        };
        self.cloud_processing = store.cloud_derivative_state(
            self.source,
            &self.id,
            &preview.source_revision,
            &preview.preview_hash,
        )?;
        Ok(self)
    }
}
