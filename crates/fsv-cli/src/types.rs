use clap::ValueEnum;

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Table,
    Json,
    Csv,
    Tsv,
}

/// `reviewed_cli.ReviewOutput`: reviewed commands whose review and readback
/// are JSON only.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewOutput {
    Json,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchSort {
    Relevance,
    Created,
    Modified,
}

impl SearchSort {
    pub fn as_str(self) -> &'static str {
        match self {
            SearchSort::Relevance => "relevance",
            SearchSort::Created => "created",
            SearchSort::Modified => "modified",
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder {
    Asc,
    Desc,
}

impl SortOrder {
    pub fn as_str(self) -> &'static str {
        match self {
            SortOrder::Asc => "asc",
            SortOrder::Desc => "desc",
        }
    }
}

/// Effective output format: `--json` wins over `-o`.
pub fn effective(format: OutputFormat, json_out: bool) -> OutputFormat {
    if json_out { OutputFormat::Json } else { format }
}
