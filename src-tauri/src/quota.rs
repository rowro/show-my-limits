use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub utilization: Option<f64>,
    pub reset: Option<f64>,
    pub status: Option<String>,
}

impl Window {
    pub fn remaining(&self, now: f64) -> Option<f64> {
        let used = self.utilization.filter(|v| v.is_finite() && *v >= 0.0)?;
        if self.reset.is_some_and(|v| !v.is_finite() || v <= now) {
            return None;
        }
        Some((1.0 - used).max(0.0))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub host: String,
    pub observed_at: f64,
    pub windows: BTreeMap<String, Window>,
    pub status: Option<String>,
    pub overage_reason: Option<String>,
}

impl Profile {
    pub fn title(&self, now: f64) -> String {
        let age = now - self.observed_at;
        if !(0.0..600.0).contains(&age) {
            return "—".into();
        }
        match self.windows.get("5h").and_then(|w| w.remaining(now)) {
            Some(value) => format!("{}%", (value * 100.0).ceil() as u32),
            None => "—".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub kind: String,
    pub host: String,
    pub observed_at: f64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyState {
    pub pid: u32,
    pub running: bool,
    pub updated_at: f64,
    pub started_at: f64,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    pub health: Option<Health>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quota_expiration_and_staleness() {
        let mut profile = Profile {
            id: "one".into(),
            host: "gateway.example".into(),
            observed_at: 1000.0,
            windows: BTreeMap::from([(
                "5h".into(),
                Window {
                    utilization: Some(0.75),
                    reset: Some(2000.0),
                    status: None,
                },
            )]),
            status: None,
            overage_reason: None,
        };
        assert_eq!(profile.title(1001.0), "25%");
        assert_eq!(profile.title(1601.0), "—");
        profile.observed_at = 1999.0;
        assert_eq!(profile.title(2001.0), "—");
        assert_eq!(
            Window {
                utilization: Some(1.2),
                reset: None,
                status: None
            }
            .remaining(1000.0),
            Some(0.0)
        );
        assert_eq!(
            Window {
                utilization: Some(f64::NAN),
                reset: None,
                status: None
            }
            .remaining(1000.0),
            None
        );
    }
}
