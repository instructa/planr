use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str::FromStr};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub description: Option<String>,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub project_id: String,
    pub stage: String,
    pub path: String,
    pub title: String,
    pub slug: String,
    pub parse_status: String,
    pub archived: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub project_id: String,
    pub parent_item_id: Option<String>,
    pub title: String,
    pub description: String,
    pub status: ItemStatus,
    pub work_type: WorkType,
    pub priority: i64,
    pub worker_id: Option<String>,
    pub plan_path: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Pending,
    Ready,
    Picked,
    Running,
    InReview,
    Blocked,
    Closed,
    ClosedPartial,
    Failed,
    Cancelled,
}

impl ItemStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Picked => "picked",
            Self::Running => "running",
            Self::InReview => "in_review",
            Self::Blocked => "blocked",
            Self::Closed => "closed",
            Self::ClosedPartial => "closed_partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub const fn is_settled(self) -> bool {
        matches!(self, Self::Closed | Self::ClosedPartial | Self::Cancelled)
    }
}

impl fmt::Display for ItemStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ItemStatus {
    type Err = VocabularyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "ready" => Ok(Self::Ready),
            "picked" => Ok(Self::Picked),
            "running" => Ok(Self::Running),
            "in_review" => Ok(Self::InReview),
            "blocked" => Ok(Self::Blocked),
            "closed" => Ok(Self::Closed),
            "closed_partial" => Ok(Self::ClosedPartial),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(VocabularyError::new("item status", value)),
        }
    }
}

impl TryFrom<&str> for ItemStatus {
    type Error = VocabularyError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl PartialEq<&str> for ItemStatus {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkType {
    Generic,
    Research,
    Plan,
    Code,
    Review,
    Fix,
    Test,
    Shell,
    Release,
    Other(String),
}

impl WorkType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Generic => "generic",
            Self::Research => "research",
            Self::Plan => "plan",
            Self::Code => "code",
            Self::Review => "review",
            Self::Fix => "fix",
            Self::Test => "test",
            Self::Shell => "shell",
            Self::Release => "release",
            Self::Other(value) => value.as_str(),
        }
    }
}

impl fmt::Display for WorkType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for WorkType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WorkType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self::from(value))
    }
}

impl FromStr for WorkType {
    type Err = VocabularyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self::from(value.to_string()))
    }
}

impl From<String> for WorkType {
    fn from(value: String) -> Self {
        match value.as_str() {
            "generic" => Self::Generic,
            "research" => Self::Research,
            "plan" => Self::Plan,
            "code" => Self::Code,
            "review" => Self::Review,
            "fix" => Self::Fix,
            "test" => Self::Test,
            "shell" => Self::Shell,
            "release" => Self::Release,
            _ => Self::Other(value),
        }
    }
}

impl TryFrom<&str> for WorkType {
    type Error = VocabularyError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl PartialEq<&str> for WorkType {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Blocks,
    HandsTo,
    Reviews,
    RelatesTo,
}

impl LinkKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Blocks => "blocks",
            Self::HandsTo => "hands_to",
            Self::Reviews => "reviews",
            Self::RelatesTo => "relates_to",
        }
    }

    pub const fn blocks_readiness(self) -> bool {
        matches!(self, Self::Blocks | Self::HandsTo)
    }
}

impl fmt::Display for LinkKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for LinkKind {
    type Err = VocabularyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "blocks" => Ok(Self::Blocks),
            "hands_to" => Ok(Self::HandsTo),
            "reviews" => Ok(Self::Reviews),
            "relates_to" => Ok(Self::RelatesTo),
            _ => Err(VocabularyError::new("link kind", value)),
        }
    }
}

impl TryFrom<&str> for LinkKind {
    type Error = VocabularyError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Requested,
    Approved,
    Denied,
}

impl ApprovalStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Approved => "approved",
            Self::Denied => "denied",
        }
    }
}

impl fmt::Display for ApprovalStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ApprovalStatus {
    type Err = VocabularyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "requested" => Ok(Self::Requested),
            "approved" => Ok(Self::Approved),
            "denied" => Ok(Self::Denied),
            _ => Err(VocabularyError::new("approval status", value)),
        }
    }
}

impl TryFrom<&str> for ApprovalStatus {
    type Error = VocabularyError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VocabularyError {
    kind: &'static str,
    value: String,
}

impl VocabularyError {
    fn new(kind: &'static str, value: &str) -> Self {
        Self {
            kind,
            value: value.to_string(),
        }
    }
}

impl fmt::Display for VocabularyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown {}: {}", self.kind, self.value)
    }
}

impl std::error::Error for VocabularyError {}
