use crate::domain::common::Value;
use crate::domain::query::QueryResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartKind {
    Line,
    Bar,
    Pie,
    Doughnut,
    Scatter,
}

impl ChartKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChartKind::Line => "line",
            ChartKind::Bar => "bar",
            ChartKind::Pie => "pie",
            ChartKind::Doughnut => "doughnut",
            ChartKind::Scatter => "scatter",
        }
    }
}

/// Declarative chart description from the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartDefinition {
    pub title: String,
    pub kind: ChartKind,
    /// Column used for the labels (or X axis for scatter charts).
    pub x: String,
    /// Columns plotted as series.
    pub y: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DatasetData {
    Values(Vec<Option<f64>>),
    Points(Vec<(f64, f64)>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dataset {
    pub label: String,
    pub data: DatasetData,
}

/// Chart data prepared from a query result.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartData {
    pub title: String,
    pub kind: ChartKind,
    pub labels: Vec<String>,
    pub datasets: Vec<Dataset>,
}

impl ChartDefinition {
    /// Extracts the series from a result set; the error is a user-facing message.
    pub fn build(&self, result: &QueryResult) -> Result<ChartData, String> {
        if self.y.is_empty() {
            return Err("chart has no 'y' columns".into());
        }
        if result.rows.is_empty() {
            // Nothing to draw (and an empty result carries no column names).
            return Ok(ChartData {
                title: self.title.clone(),
                kind: self.kind,
                labels: vec![],
                datasets: self
                    .y
                    .iter()
                    .map(|label| Dataset {
                        label: label.clone(),
                        data: DatasetData::Values(vec![]),
                    })
                    .collect(),
            });
        }
        let x_idx = result
            .column_index(&self.x)
            .ok_or_else(|| format!("column '{}' not found in the result", self.x))?;
        let y_idx = self
            .y
            .iter()
            .map(|c| {
                result
                    .column_index(c)
                    .map(|i| (c.clone(), i))
                    .ok_or_else(|| format!("column '{c}' not found in the result"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let labels: Vec<String> = result.rows.iter().map(|r| r[x_idx].display()).collect();

        let datasets = if self.kind == ChartKind::Scatter {
            y_idx
                .into_iter()
                .map(|(label, yi)| {
                    let points = result
                        .rows
                        .iter()
                        .filter_map(|r| Some((r[x_idx].as_f64()?, r[yi].as_f64()?)))
                        .collect();
                    Dataset {
                        label,
                        data: DatasetData::Points(points),
                    }
                })
                .collect()
        } else {
            y_idx
                .into_iter()
                .map(|(label, yi)| Dataset {
                    label,
                    data: DatasetData::Values(result.rows.iter().map(|r| number(&r[yi])).collect()),
                })
                .collect()
        };

        Ok(ChartData {
            title: self.title.clone(),
            kind: self.kind,
            labels: if self.kind == ChartKind::Scatter {
                vec![]
            } else {
                labels
            },
            datasets,
        })
    }
}

fn number(v: &Value) -> Option<f64> {
    v.as_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result() -> QueryResult {
        QueryResult {
            columns: vec!["region".into(), "total".into(), "orders".into()],
            rows: vec![
                vec![Value::Text("EU".into()), Value::Float(10.5), Value::Int(3)],
                vec![Value::Text("US".into()), Value::Null, Value::Int(4)],
            ],
            truncated: false,
        }
    }

    fn def(kind: ChartKind) -> ChartDefinition {
        ChartDefinition {
            title: "t".into(),
            kind,
            x: "region".into(),
            y: vec!["total".into(), "orders".into()],
        }
    }

    #[test]
    fn builds_category_series() {
        let data = def(ChartKind::Bar).build(&result()).unwrap();
        assert_eq!(data.labels, vec!["EU", "US"]);
        assert_eq!(data.datasets.len(), 2);
        assert_eq!(
            data.datasets[0].data,
            DatasetData::Values(vec![Some(10.5), None])
        );
        assert_eq!(
            data.datasets[1].data,
            DatasetData::Values(vec![Some(3.0), Some(4.0)])
        );
    }

    #[test]
    fn builds_scatter_points_skipping_non_numeric() {
        let mut r = result();
        r.columns[0] = "x".into();
        r.rows[0][0] = Value::Float(1.0);
        r.rows[1][0] = Value::Text("n/a".into());
        let mut d = def(ChartKind::Scatter);
        d.x = "x".into();
        d.y = vec!["orders".into()];
        let data = d.build(&r).unwrap();
        assert!(data.labels.is_empty());
        assert_eq!(data.datasets[0].data, DatasetData::Points(vec![(1.0, 3.0)]));
    }

    #[test]
    fn reports_missing_columns() {
        let mut d = def(ChartKind::Line);
        d.x = "nope".into();
        assert!(d.build(&result()).unwrap_err().contains("'nope'"));
        let mut d = def(ChartKind::Line);
        d.y = vec!["ghost".into()];
        assert!(d.build(&result()).unwrap_err().contains("'ghost'"));
    }

    #[test]
    fn empty_result_yields_empty_chart() {
        let data = def(ChartKind::Bar).build(&QueryResult::default()).unwrap();
        assert!(data.labels.is_empty());
        assert_eq!(data.datasets.len(), 2);
    }

    #[test]
    fn rejects_empty_y() {
        let mut d = def(ChartKind::Line);
        d.y.clear();
        assert!(d.build(&result()).is_err());
    }

    #[test]
    fn kind_names() {
        assert_eq!(ChartKind::Doughnut.as_str(), "doughnut");
        assert_eq!(ChartKind::Scatter.as_str(), "scatter");
    }
}
