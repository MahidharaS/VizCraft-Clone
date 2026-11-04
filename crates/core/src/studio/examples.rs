//! Example workflows on the Retail sample (Sales joined with Product).

use serde_json::{Value as J, json};

use super::StudioWorkflow;

pub const EXAMPLES: [(&str, &str); 4] = [
    ("high_margin", "Predict high-margin orders"),
    ("ols", "Explain sales with OLS"),
    ("arima", "Forecast monthly sales with ARIMA"),
    ("segments", "Segment orders (clustering)"),
];

fn set(wf: &mut StudioWorkflow, id: u64, kv: &[(&str, J)]) {
    if let Some(w) = wf.widget_mut(id) {
        for (k, v) in kv {
            w.settings.insert(k.to_string(), v.clone());
        }
    }
}

fn label(wf: &mut StudioWorkflow, id: u64, l: &str) {
    if let Some(w) = wf.widget_mut(id) {
        w.label = l.to_string();
    }
}

/// Sales ⋈ Product with a revenue column: returns (workflow, id of the enriched data widget).
fn sales_with_products(name: &str, description: &str) -> (StudioWorkflow, u64) {
    let mut wf = StudioWorkflow { name: name.into(), description: description.into(), ..Default::default() };
    let sales = wf.add("data.table", 60.0, 120.0);
    set(&mut wf, sales, &[("table", json!("Sales"))]);
    label(&mut wf, sales, "Sales");
    let prod = wf.add("data.table", 60.0, 300.0);
    set(&mut wf, prod, &[("table", json!("Product"))]);
    label(&mut wf, prod, "Product");
    let merge = wf.add("data.merge", 230.0, 200.0);
    set(&mut wf, merge, &[("left_key", json!("ProductID")), ("right_key", json!("ProductID")), ("how", json!("left"))]);
    let _ = wf.connect(sales, 0, merge, 0);
    let _ = wf.connect(prod, 0, merge, 1);
    (wf, merge)
}

pub fn build(key: &str) -> Option<StudioWorkflow> {
    Some(match key {
        "high_margin" => {
            let (mut wf, merge) = sales_with_products("Predict high-margin orders", "Sales joined with Product; a high-margin flag (net margin ≥ 58%) predicted by Random Forest and Logistic Regression, cross-validated, with ROC and confusion matrix.");
            let fc = wf.add("tf.constructor", 400.0, 200.0);
            set(&mut wf, fc, &[("code", json!("Net Price = [Unit Price] * (1 - Discount)\nMargin = ([Net Price] - [Unit Cost]) / [Net Price]\nHigh margin = if(Margin >= 0.58, \"High\", \"Low\")"))]);
            let _ = wf.connect(merge, 0, fc, 0);
            let sel = wf.add("data.select_columns", 570.0, 200.0);
            set(&mut wf, sel, &[("target", json!("High margin")), ("ignore", json!(["OrderID", "Date", "ProductID", "Product", "Unit Cost", "Unit Margin", "Margin", "Net Price", "Unit Price"]))]);
            let _ = wf.connect(fc, 0, sel, 0);
            let rf = wf.add("model.forest", 760.0, 90.0);
            set(&mut wf, rf, &[("trees", json!(30)), ("max_depth", json!(8))]);
            let lr = wf.add("model.logistic", 760.0, 230.0);
            let ts = wf.add("eval.test_score", 950.0, 160.0);
            let roc = wf.add("eval.roc", 1130.0, 90.0);
            set(&mut wf, roc, &[("class", json!(0))]);
            let cm = wf.add("eval.confusion", 1130.0, 230.0);
            let rank = wf.add("eval.rank", 760.0, 380.0);
            let pred = wf.add("eval.predictions", 950.0, 380.0);
            let reg = wf.add("deploy.register", 950.0, 10.0);
            set(&mut wf, reg, &[("name", json!("high-margin-orders"))]);
            let table = wf.add("data.datatable", 570.0, 380.0);
            for (a, ap, b, bp) in [(sel, 0, rf, 0), (sel, 0, lr, 0), (sel, 0, ts, 0), (rf, 0, ts, 1), (lr, 0, ts, 1), (ts, 0, roc, 0), (ts, 0, cm, 0), (sel, 0, rank, 0), (sel, 0, pred, 0), (rf, 0, pred, 1), (rf, 0, reg, 0), (ts, 0, reg, 1), (fc, 0, table, 0)] {
                let _ = wf.connect(a, ap, b, bp);
            }
            wf
        }
        "ols" => {
            let (mut wf, merge) = sales_with_products("Explain sales with OLS", "Order revenue explained by quantity, discount, list price, channel and category: an OLS summary with p-values, plus ANOVA and a scatter plot.");
            let fc = wf.add("tf.constructor", 400.0, 200.0);
            set(&mut wf, fc, &[("code", json!("Revenue = Quantity * [Unit Price] * (1 - Discount)"))]);
            let _ = wf.connect(merge, 0, fc, 0);
            let ols = wf.add("stats.ols", 600.0, 90.0);
            set(&mut wf, ols, &[("target", json!("Revenue")), ("predictors", json!(["Quantity", "Discount", "List Price", "Channel", "Category"]))]);
            let anova = wf.add("stats.anova", 600.0, 230.0);
            set(&mut wf, anova, &[("value", json!("Revenue")), ("factor", json!("Category"))]);
            let sc = wf.add("viz.scatter", 600.0, 370.0);
            set(&mut wf, sc, &[("x", json!("List Price")), ("y", json!("Revenue")), ("color", json!("Category"))]);
            let bx = wf.add("viz.box", 780.0, 300.0);
            set(&mut wf, bx, &[("column", json!("Revenue")), ("group", json!("Channel"))]);
            let dist = wf.add("viz.distributions", 780.0, 160.0);
            set(&mut wf, dist, &[("column", json!("Revenue")), ("split", json!("Channel"))]);
            let corr = wf.add("viz.correlations", 400.0, 370.0);
            for b in [ols, anova, sc, bx, dist, corr] {
                let _ = wf.connect(fc, 0, b, 0);
            }
            let tt = wf.add("stats.ttest", 780.0, 20.0);
            set(&mut wf, tt, &[("column", json!("Revenue")), ("group", json!("Channel")), ("a", json!("Online")), ("b", json!("Store"))]);
            let _ = wf.connect(fc, 0, tt, 0);
            wf
        }
        "arima" => {
            let mut wf = StudioWorkflow { name: "Forecast monthly sales with ARIMA".into(), description: "Monthly revenue as a time series: decomposition, autocorrelation, ARIMA and Holt–Winters forecasts.".into(), ..Default::default() };
            let sales = wf.add("data.table", 60.0, 200.0);
            set(&mut wf, sales, &[("table", json!("Sales"))]);
            label(&mut wf, sales, "Sales");
            let fc = wf.add("tf.constructor", 220.0, 200.0);
            set(&mut wf, fc, &[("code", json!("Revenue = Quantity * [Unit Price] * (1 - Discount)"))]);
            let ta = wf.add("tf.timeagg", 380.0, 200.0);
            set(&mut wf, ta, &[("date", json!("Date")), ("value", json!("Revenue")), ("period", json!("month")), ("agg", json!("sum"))]);
            let ar = wf.add("stats.arima", 560.0, 90.0);
            set(&mut wf, ar, &[("value", json!("Revenue")), ("time", json!("Period")), ("p", json!(1)), ("d", json!(1)), ("q", json!(0)), ("horizon", json!(6))]);
            let ets = wf.add("stats.ets", 560.0, 220.0);
            set(&mut wf, ets, &[("value", json!("Revenue")), ("method", json!("holt_winters")), ("period", json!(12)), ("horizon", json!(6))]);
            let dec = wf.add("stats.decompose", 560.0, 350.0);
            set(&mut wf, dec, &[("value", json!("Revenue")), ("period", json!(12))]);
            let acf = wf.add("stats.acf", 740.0, 350.0);
            set(&mut wf, acf, &[("value", json!("Revenue")), ("lags", json!(12))]);
            let line = wf.add("viz.line", 740.0, 90.0);
            set(&mut wf, line, &[("x", json!("Period")), ("y", json!(["Revenue", "Forecast", "Lower", "Upper"]))]);
            for (a, b) in [(sales, fc), (fc, ta), (ta, ar), (ta, ets), (ta, dec), (ta, acf), (ar, line)] {
                let _ = wf.connect(a, 0, b, 0);
            }
            wf
        }
        "segments" => {
            let (mut wf, merge) = sales_with_products("Segment orders (clustering)", "k-means and hierarchical clustering of orders by price, quantity and discount, with silhouette, PCA and t-SNE projections.");
            let fc = wf.add("tf.constructor", 400.0, 200.0);
            set(&mut wf, fc, &[("code", json!("Revenue = Quantity * [Unit Price] * (1 - Discount)"))]);
            let _ = wf.connect(merge, 0, fc, 0);
            let sample = wf.add("data.sample", 560.0, 200.0);
            set(&mut wf, sample, &[("mode", json!("size")), ("size", json!(400)), ("stratify", json!(false))]);
            let _ = wf.connect(fc, 0, sample, 0);
            let cols = json!(["Quantity", "Discount", "List Price", "Revenue"]);
            let km = wf.add("unsup.kmeans", 730.0, 90.0);
            set(&mut wf, km, &[("k", json!(4)), ("columns", cols.clone())]);
            let hc = wf.add("unsup.hclust", 730.0, 230.0);
            set(&mut wf, hc, &[("k", json!(4)), ("columns", cols.clone()), ("max_rows", json!(120)), ("label", json!("Product"))]);
            let pca = wf.add("unsup.pca", 730.0, 370.0);
            set(&mut wf, pca, &[("columns", cols.clone())]);
            let sil = wf.add("unsup.silhouette", 910.0, 90.0);
            set(&mut wf, sil, &[("cluster", json!("Cluster")), ("columns", cols.clone())]);
            let ts = wf.add("unsup.tsne", 910.0, 230.0);
            set(&mut wf, ts, &[("columns", cols.clone()), ("max_rows", json!(300))]);
            let sc = wf.add("viz.scatter", 1090.0, 230.0);
            set(&mut wf, sc, &[("x", json!("t-SNE-x")), ("y", json!("t-SNE-y")), ("color", json!("Cluster")), ("regression", json!(false))]);
            let hm = wf.add("viz.heatmap", 910.0, 370.0);
            set(&mut wf, hm, &[("columns", cols)]);
            for (a, b) in [(sample, km), (sample, hc), (sample, pca), (km, sil), (km, ts), (ts, sc), (km, hm)] {
                let _ = wf.connect(a, 0, b, 0);
            }
            wf
        }
        _ => return None,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::studio::exec::{Ctx, Executor, State};
    use std::rc::Rc;

    #[test]
    fn examples_run_on_the_sample() {
        let p = crate::sample::sales();
        let built = crate::build::build(&p, &vizcraft_query::NoHost, &Default::default());
        let tables: std::collections::BTreeMap<String, Rc<crate::build::TableData>> = built.tables.clone();
        assert!(tables.contains_key("Sales") && tables.contains_key("Product"), "{:?}", tables.keys().collect::<Vec<_>>());
        let ctx = Ctx { tables: &tables, generation: 1, registry: &[], base_dir: None };
        for (key, _) in EXAMPLES {
            let wf = build(key).unwrap();
            let mut ex = Executor::default();
            ex.run(&ctx, &wf);
            for w in &wf.widgets {
                let o = ex.get(w.id).unwrap();
                assert!(matches!(o.state, State::Ok | State::Warning), "{key} / {}: {:?} {}", w.label, o.state, o.message);
            }
        }
    }

}
