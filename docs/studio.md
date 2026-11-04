# Data Science Studio

An Orange-style visual workbench for machine learning and statistics, inside VizardCraft. Open it
from the rail (the wired-nodes icon) or **Home → AI → Studio**.

## Layout

| Area | What it does |
| --- | --- |
| Tabs (top) | One tab per workflow. **+ New** → empty workflow or an example. **Fit** fits the canvas; Ctrl + wheel zooms. **Serve models** starts the built-in model server. |
| Palette (left) | Widgets by category: Data, Transform, Model, Evaluate, Visualize, Unsupervised, Statistics, Deploy. Click to add; it connects to the selected widget when the channels fit. |
| Canvas | Drag widgets to move them, drag from an output port (right) to an input port (left) to connect. Right-click a widget to delete it or disconnect inputs; `Delete` removes the selection. Port colours: blue = Data, orange = Model, pink = Evaluation results. |
| Settings (right) | The selected widget's parameters (columns, learners, folds, …). With nothing selected: workflow name and the model registry. |
| Window (bottom) | The widget's output: data table, regression report, score table, confusion matrix, ROC / lift / calibration curves, scatter, distributions, box plot, heat map, correlations, sieve / mosaic, line chart, dendrogram, silhouette, PCA, forecasts, decomposition, ACF/PACF, distribution fits, drift report. |

Everything recomputes automatically and is cached per widget, so only changed parts rerun.

## Examples (Retail sample)

- **Predict high-margin orders**: Sales ⋈ Product → derived columns → Random Forest and Logistic
  Regression → Test & Score (stratified 5-fold) → ROC, confusion matrix, rank, predictions,
  Register Model.
- **Explain sales with OLS**: OLS with full inference, ANOVA, t-test, distributions, box plot,
  scatter, correlations.
- **Forecast monthly sales with ARIMA**: monthly aggregate → ARIMA, Holt-Winters, seasonal
  decomposition, autocorrelation, line chart with forecast band.
- **Segment orders (clustering)**: sample → k-Means, hierarchical clustering, t-SNE, PCA,
  silhouette, heat map.

## MLOps

1. Add **Deploy → Register Model** after a trained model (optionally also connect Test & Score so
   the cross-validated metrics are stored). Set a name and click **Register new version**.
2. Versions are saved in the project and in `~/.vizardcraft/models/<name>/v<N>.json`. The registry
   panel lets you set stages (Staging, Production, Archived; promoting to Production archives the
   previous Production version) and **Export** JSON plus a standalone Python scorer to
   `~/.vizardcraft/models/exports/`.
3. **Batch Score** applies a registered model to new data; **Drift Monitor** compares new data to
   the training data (PSI, KS); **Save as Table** saves scored data as a model table for reports.
4. **Serve models** (top right) or the standalone binary serves the registry over HTTP:

```bash
vizardcraft-mlserve --port 8765
```
```bash
curl -X POST http://127.0.0.1:8765/models/high-margin-orders/predict \
  -H 'Content-Type: application/json' \
  -d '{"rows":[{"Quantity":2,"Discount":0,"Channel":"Store","Category":"Apparel","List Price":45,"StoreID":"S01"}]}'
```

`GET /models` lists models and versions; the served version is Production, else Staging, else the
latest. `vizardcraft-mlserve` ships next to the app in every package (macOS: inside
`VizardCraft.app/Contents/MacOS/`).

## Algorithms

All original Rust implementations (Orange is GPL; only its user experience was studied).

- Supervised: kNN, decision tree, random forest, SVM (linear), linear / ridge / lasso / elastic net,
  logistic regression, naive Bayes, neural network (MLP), gradient boosting, AdaBoost.
- Evaluation: cross-validation, random sampling, leave-one-out, test on train / separate data;
  AUC, CA, F1, precision, recall, log loss, MCC, MSE, RMSE, MAE, R²; ROC, lift, gains,
  calibration, Brier; permutation importance, per-prediction explanations; feature ranking.
- Unsupervised: k-means (with silhouette-based k), hierarchical clustering, DBSCAN, PCA, MDS, t-SNE.
- Statistics: descriptive statistics, OLS with full inference, logit, Poisson GLM, ANOVA with
  Tukey HSD, χ², correlation tests, normality tests, t-tests, Mann-Whitney / Kruskal-Wallis,
  seasonal decomposition, ACF/PACF, ARIMA, exponential smoothing, distribution fitting.
