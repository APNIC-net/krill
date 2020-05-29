use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

use crate::commons::api::RoaDefinition;
use crate::commons::bgp::Announcement;

//------------ BgpAnalysisReport -------------------------------------------

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct BgpAnalysisReport(Vec<BgpAnalysisEntry>);

impl BgpAnalysisReport {
    pub fn new(mut roas: Vec<BgpAnalysisEntry>) -> Self {
        roas.sort();
        BgpAnalysisReport(roas)
    }

    pub fn entries(&self) -> &Vec<BgpAnalysisEntry> {
        &self.0
    }

    pub fn matching_defs(&self, state: BgpAnalysisState) -> Vec<&RoaDefinition> {
        self.matching_entries(state)
            .into_iter()
            .map(|e| &e.definition)
            .collect()
    }

    pub fn matching_entries(&self, state: BgpAnalysisState) -> Vec<&BgpAnalysisEntry> {
        self.0.iter().filter(|e| e.state == state).collect()
    }
}

impl fmt::Display for BgpAnalysisReport {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let entries = self.entries();

        let mut entry_map: HashMap<BgpAnalysisState, Vec<&BgpAnalysisEntry>> = HashMap::new();
        for entry in entries.into_iter() {
            let state = entry.state();
            if !entry_map.contains_key(&state) {
                entry_map.insert(state, vec![]);
            }
            entry_map.get_mut(&state).unwrap().push(entry);
        }

        if entry_map.contains_key(&BgpAnalysisState::RoaNoAnnouncementInfo) {
            write!(f, "no BGP announcements known")
        } else {
            if let Some(authorizing) = entry_map.get(&BgpAnalysisState::RoaAuthorizing) {
                writeln!(f, "Authorizations causing VALID announcements:")?;
                for roa in authorizing {
                    writeln!(f)?;
                    writeln!(f, "\tDefinition: {}", roa.definition)?;
                    writeln!(f)?;
                    writeln!(f, "\t\tAuthorizes:")?;
                    for ann in roa.authorizes.iter() {
                        writeln!(f, "\t\t{}", ann)?;
                    }

                    if !roa.disallows.is_empty() {
                        writeln!(f)?;
                        writeln!(f, "\t\tDisallows:")?;
                        for ann in roa.disallows.iter() {
                            writeln!(f, "\t\t{}", ann)?;
                        }
                    }
                }
                writeln!(f)?;
            }

            if let Some(disallowing) = entry_map.get(&BgpAnalysisState::RoaDisallowing) {
                writeln!(f, "Authorizations causing INVALID announcements only:")?;
                for roa in disallowing {
                    writeln!(f)?;
                    writeln!(f, "\tDefinition: {}", roa.definition)?;
                    writeln!(f)?;
                    writeln!(f, "\t\tDisallows:")?;
                    for ann in roa.disallows.iter() {
                        writeln!(f, "\t\t{}", ann)?;
                    }
                }
                writeln!(f)?;
            }

            if let Some(stales) = entry_map.get(&BgpAnalysisState::RoaStale) {
                writeln!(
                    f,
                    "Authorizations for which no announcements are found (possibly stale):"
                )?;
                writeln!(f)?;
                for roa in stales {
                    writeln!(f, "\tDefinition: {}", roa.definition)?;
                }
                writeln!(f)?;
            }

            if let Some(valids) = entry_map.get(&BgpAnalysisState::AnnouncementValid) {
                writeln!(f, "Announcements which are valid:")?;
                writeln!(f)?;
                for ann in valids {
                    writeln!(f, "\tAnnouncement: {}", ann.definition)?;
                }
                writeln!(f)?;
            }

            if let Some(invalid_asn) = entry_map.get(&BgpAnalysisState::AnnouncementInvalidAsn) {
                writeln!(f, "Announcements from an unauthorized ASN:")?;
                for ann in invalid_asn {
                    writeln!(f)?;
                    writeln!(f, "\tAnnouncement: {}", ann.definition)?;
                    writeln!(f)?;
                    writeln!(f, "\t\tDisallowed by authorization(s):")?;
                    for roa in ann.disallowed_by.iter() {
                        writeln!(f, "\t\t{}", roa)?;
                    }
                }
                writeln!(f)?;
            }

            if let Some(invalid_length) =
                entry_map.get(&BgpAnalysisState::AnnouncementInvalidLength)
            {
                writeln!(f, "Announcements from an authorized ASN, which are too specific (not allowed by max length):")?;
                for ann in invalid_length {
                    writeln!(f)?;
                    writeln!(f, "\tAnnouncement: {}", ann.definition)?;
                    writeln!(f)?;
                    writeln!(f, "\t\tDisallowed by authorization(s):")?;
                    for roa in ann.disallowed_by.iter() {
                        writeln!(f, "\t\t{}", roa)?;
                    }
                }
                writeln!(f)?;
            }

            if let Some(not_found) = entry_map.get(&BgpAnalysisState::AnnouncementNotFound) {
                writeln!(f, "Announcements which are 'not found' (not covered by any of your authorizations):")?;
                writeln!(f)?;
                for ann in not_found {
                    writeln!(f, "\tAnnouncement: {}", ann.definition)?;
                }
                writeln!(f)?;
            }

            Ok(())
        }
    }
}

//------------ BgpAnalysisEntry --------------------------------------------

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct BgpAnalysisEntry {
    #[serde(flatten)]
    definition: RoaDefinition,
    state: BgpAnalysisState,
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed_by: Option<RoaDefinition>,
    #[serde(skip_serializing_if = "Vec::is_empty", default = "Vec::new")]
    disallowed_by: Vec<RoaDefinition>,
    #[serde(skip_serializing_if = "Vec::is_empty", default = "Vec::new")]
    authorizes: Vec<Announcement>,
    #[serde(skip_serializing_if = "Vec::is_empty", default = "Vec::new")]
    disallows: Vec<Announcement>,
}

impl BgpAnalysisEntry {
    pub fn definition(&self) -> &RoaDefinition {
        &self.definition
    }

    pub fn state(&self) -> BgpAnalysisState {
        self.state
    }

    pub fn allowed_by(&self) -> Option<&RoaDefinition> {
        self.allowed_by.as_ref()
    }

    pub fn disallowed_by(&self) -> &Vec<RoaDefinition> {
        &self.disallowed_by
    }

    pub fn authorizes(&self) -> &Vec<Announcement> {
        &self.authorizes
    }

    pub fn disallows(&self) -> &Vec<Announcement> {
        &self.disallows
    }

    pub fn roa_authorizing(
        definition: RoaDefinition,
        mut authorizes: Vec<Announcement>,
        mut disallows: Vec<Announcement>,
    ) -> Self {
        authorizes.sort();
        disallows.sort();
        BgpAnalysisEntry {
            definition,
            state: BgpAnalysisState::RoaAuthorizing,
            allowed_by: None,
            disallowed_by: vec![],
            authorizes,
            disallows,
        }
    }

    pub fn roa_disallowing(definition: RoaDefinition, mut disallows: Vec<Announcement>) -> Self {
        disallows.sort();
        BgpAnalysisEntry {
            definition,
            state: BgpAnalysisState::RoaDisallowing,
            allowed_by: None,
            disallowed_by: vec![],
            authorizes: vec![],
            disallows,
        }
    }

    pub fn roa_stale(definition: RoaDefinition) -> Self {
        BgpAnalysisEntry {
            definition,
            state: BgpAnalysisState::RoaStale,
            allowed_by: None,
            disallowed_by: vec![],
            authorizes: vec![],
            disallows: vec![],
        }
    }

    pub fn roa_no_announcement_info(definition: RoaDefinition) -> Self {
        BgpAnalysisEntry {
            definition,
            state: BgpAnalysisState::RoaNoAnnouncementInfo,
            allowed_by: None,
            disallowed_by: vec![],
            authorizes: vec![],
            disallows: vec![],
        }
    }

    pub fn announcement_valid(announcement: Announcement, allowed_by: RoaDefinition) -> Self {
        BgpAnalysisEntry {
            definition: RoaDefinition::from(announcement),
            state: BgpAnalysisState::AnnouncementValid,
            allowed_by: Some(allowed_by),
            disallowed_by: vec![],
            authorizes: vec![],
            disallows: vec![],
        }
    }

    pub fn announcement_invalid_asn(
        announcement: Announcement,
        mut disallowed_by: Vec<RoaDefinition>,
    ) -> Self {
        disallowed_by.sort();
        BgpAnalysisEntry {
            definition: RoaDefinition::from(announcement),
            state: BgpAnalysisState::AnnouncementInvalidAsn,
            allowed_by: None,
            disallowed_by,
            authorizes: vec![],
            disallows: vec![],
        }
    }

    pub fn announcement_invalid_length(
        announcement: Announcement,
        mut disallowed_by: Vec<RoaDefinition>,
    ) -> Self {
        disallowed_by.sort();
        BgpAnalysisEntry {
            definition: RoaDefinition::from(announcement),
            state: BgpAnalysisState::AnnouncementInvalidLength,
            allowed_by: None,
            disallowed_by,
            authorizes: vec![],
            disallows: vec![],
        }
    }

    pub fn announcement_not_found(announcement: Announcement) -> Self {
        BgpAnalysisEntry {
            definition: RoaDefinition::from(announcement),
            state: BgpAnalysisState::AnnouncementNotFound,
            allowed_by: None,
            disallowed_by: vec![],
            authorizes: vec![],
            disallows: vec![],
        }
    }
}

impl Ord for BgpAnalysisEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        let mut ordering = self.state.cmp(&other.state);
        if ordering == Ordering::Equal {
            ordering = self.definition.cmp(&other.definition);
        }
        ordering
    }
}

impl PartialOrd for BgpAnalysisEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

//------------ BgpAnalysisState --------------------------------------------

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialOrd, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BgpAnalysisState {
    RoaAuthorizing,
    RoaDisallowing,
    RoaStale,
    AnnouncementValid,
    AnnouncementInvalidLength,
    AnnouncementInvalidAsn,
    AnnouncementNotFound,
    RoaNoAnnouncementInfo,
}

//------------ AnnouncementReport ------------------------------------------

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct AnnouncementReport(Vec<AnnouncementReportEntry>);

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct AnnouncementReportEntry {
    definition: RoaDefinition,
    state: AnnouncementReportState,
}

impl fmt::Display for AnnouncementReportEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let state_str = match self.state {
            AnnouncementReportState::Valid => "announcement 'valid'",
            AnnouncementReportState::InvalidAsn => "announcement 'invalid': unauthorized asn",
            AnnouncementReportState::InvalidLength => {
                "announcement 'invalid': more specific than allowed"
            }
            AnnouncementReportState::NotFound => {
                "announcement 'not found': not covered by your ROAs"
            }
            AnnouncementReportState::Stale => {
                "ROA does not cover any known announcement (stale or backup?)"
            }
            AnnouncementReportState::NoInfo => "ROA exists, but no bgp info currently available",
        };
        write!(f, "{}\t{}", self.definition, state_str)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnouncementReportState {
    Valid,
    InvalidAsn,
    InvalidLength,
    NotFound,
    Stale,
    NoInfo,
}

impl From<BgpAnalysisReport> for AnnouncementReport {
    fn from(table: BgpAnalysisReport) -> Self {
        let mut entries: Vec<AnnouncementReportEntry> = vec![];
        for def in table.matching_defs(BgpAnalysisState::AnnouncementValid) {
            entries.push(AnnouncementReportEntry {
                definition: def.clone(),
                state: AnnouncementReportState::Valid,
            })
        }

        for def in table.matching_defs(BgpAnalysisState::AnnouncementInvalidAsn) {
            entries.push(AnnouncementReportEntry {
                definition: def.clone(),
                state: AnnouncementReportState::InvalidAsn,
            })
        }

        for def in table.matching_defs(BgpAnalysisState::AnnouncementInvalidLength) {
            entries.push(AnnouncementReportEntry {
                definition: def.clone(),
                state: AnnouncementReportState::InvalidLength,
            })
        }

        for def in table.matching_defs(BgpAnalysisState::AnnouncementNotFound) {
            entries.push(AnnouncementReportEntry {
                definition: def.clone(),
                state: AnnouncementReportState::NotFound,
            })
        }
        for def in table.matching_defs(BgpAnalysisState::RoaStale) {
            entries.push(AnnouncementReportEntry {
                definition: def.clone(),
                state: AnnouncementReportState::Stale,
            })
        }
        for def in table.matching_defs(BgpAnalysisState::RoaNoAnnouncementInfo) {
            entries.push(AnnouncementReportEntry {
                definition: def.clone(),
                state: AnnouncementReportState::NoInfo,
            })
        }
        AnnouncementReport(entries)
    }
}

impl fmt::Display for AnnouncementReport {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for e in self.0.iter() {
            writeln!(f, "{}", e)?;
        }
        Ok(())
    }
}

//------------ Tests --------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_bgp_report() {
        let json = include_str!("../../../test-resources/bgp/expected_bgp_analyis_report.json");
        let report: BgpAnalysisReport = serde_json::from_str(json).unwrap();

        let expected = include_str!("../../../test-resources/bgp/expected_bgp_analysis_report.txt");

        print!("{}", report);

        assert_eq!(report.to_string(), expected);
    }

    #[test]
    fn print_roa_table_summary() {
        let json = include_str!("../../../test-resources/bgp/expected_bgp_analyis_report.json");
        let report: BgpAnalysisReport = serde_json::from_str(json).unwrap();
        let summary: AnnouncementReport = report.into();

        let expected_text = include_str!("../../../test-resources/bgp/expected_summary.txt");
        assert_eq!(summary.to_string(), expected_text);
    }
}
