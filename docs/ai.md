# AI in VizardCraft

VizardCraft has four AI surfaces:

- **Copilot**: a chat pane next to the Report, Table, Model and DAX views (Home ribbon > AI > Copilot).
  It reads the model, runs DAX and M, builds visuals, writes measures, adds filters and pages,
  and explains numbers. It asks before it changes the report, and Ctrl+Z undoes any change.
- **Formulator**: a view based on Microsoft Data Formulator, on the rail with the `formulator` key.
  You build charts from concept encodings plus natural language, and VizardCraft derives any new fields.
- **AI hub**: settings for providers and keys, local models, the OpenJEV decision model, the personal brain and privacy. It is on the rail with the `ai` key.
- **Suggestion chips**: the personal brain's recommendations, shown in the Visualizations pane's Build tab.
  An example: "Recommended: Line chart. You used a line chart for a date and a measure 82% of the time."

The engine is the `vizardcraft-ai` crate (`crates/ai`). The UI is `apps/vizardcraft/src/ai/`.

## Why it's all Rust

LangChain, LangSmith, AutoGen and CrewAI are Python libraries. VizardCraft is a single native
binary, so `vizardcraft-ai` implements the same pieces natively:

| Python tool | `vizardcraft-ai` equivalent |
|---|---|
| LangChain chat models | `provider::ChatProvider` (OpenAI-compatible, Anthropic, Gemini) |
| LangChain tools / function calling | `tools::specs()` plus `tools::execute()` |
| AgentExecutor / AutoGen loop | `agent::AgentRun` (native tool calls, or a JSON tool protocol for small models) |
| LangSmith traces | `agent::Trace`: every model call and tool call with input, output, latency and tokens, shown in the Copilot's **Traces** window |
| Memory / retrievers | `brain::memory` (BM25, optional embeddings) |

HTTP uses `ureq` with rustls and always runs on background threads. The UI never waits on the network.
Bad responses become `AiError` values and never cause a panic.

## Providers: bring your own key, or run a model locally

| Provider | Wire protocol | Default endpoint | Key |
|---|---|---|---|
| OpenAI | chat completions | `https://api.openai.com/v1` | `OPENAI_API_KEY` |
| Anthropic | Messages API | `https://api.anthropic.com` | `ANTHROPIC_API_KEY` |
| Google Gemini | generateContent | `https://generativelanguage.googleapis.com` | `GEMINI_API_KEY` |
| Hugging Face Inference | OpenAI-compatible router | `https://router.huggingface.co/v1` | `HF_TOKEN` |
| Azure OpenAI | chat completions + `api-version` | your deployment URL | `AZURE_OPENAI_API_KEY` |
| Groq / OpenRouter / Together | OpenAI-compatible | provider URLs | `GROQ_API_KEY` / `OPENROUTER_API_KEY` / `TOGETHER_API_KEY` |
| Ollama | OpenAI-compatible | `http://localhost:11434/v1` | none |
| LM Studio | OpenAI-compatible | `http://localhost:1234/v1` | none |
| llama.cpp server | OpenAI-compatible | `http://localhost:8080/v1` | none |
| vLLM | OpenAI-compatible | `http://localhost:8000/v1` | none |
| Custom | OpenAI-compatible | any | optional |

Every provider gets a current default model ID, and you can change it. For example, Anthropic
defaults to `claude-sonnet-5-5` and OpenAI to `gpt-5`. Tool calls use each API's native format.
For small local models without tool support, turn off **Native function calling**. The agent then uses
a JSON tool protocol: one JSON object per turn, either `{"tool": …, "arguments": …}` or `{"final": …}`.
Streaming isn't used, because each agent step needs the full answer.

**Local detection.** AI hub > Local models > **Scan** checks these servers in parallel, each with a timeout under 1 s:

- Ollama (`/api/tags`)
- LM Studio, llama.cpp and vLLM (`/v1/models`)
- the OpenJEV helper (`/v1/version`, ports 3000 and 8765)

The scan lists each server's models. **Use** sets that server as the active provider.

**Settings and keys.** Settings are saved in `~/.vizardcraft/ai.json` with file mode 600 on Unix. They are never saved in a
`.vizc` project, so sharing a project never shares a key. A key you type is stored in plain text in that
file. When no key is stored, VizardCraft reads the environment variable listed above, which is the better choice on shared
machines. Set `VIZARDCRAFT_HOME` to move the whole `~/.vizardcraft` folder.

**Without a provider**, everything still works in offline mode, and the UI labels it that way:

- The Copilot answers with rules: Q&A over the model, the brain's chart choice and local explanations of visuals.
  These answers still go through the agent loop, so they appear in traces and ask before applying changes.
- The Formulator uses `charts::qna::interpret`, rule-based field derivations and the brain.

## Copilot tools

`describe_model`, `run_dax`, `run_m`, `preview_table`, `create_visual`, `set_format`,
`add_measure`, `add_filter`, `create_page`, `explain_visual`, `create_query`, `suggest_relationships`.

- Tools run on the UI thread because the app owns the project. The agent thread sends each call
  over a channel and waits for the result.
- Tools that change the project are `create_visual`, `set_format`, `add_measure`, `add_filter`, `create_page`
  and `create_query`. Each one shows an **Apply / Skip** card unless "Apply changes without asking" is on.
- Every applied change records an undo checkpoint.
- `add_measure` validates the DAX before it keeps the measure.
- The brain's profile summary, a short description of your habits, is added to the system prompt.
  This lets cloud and local LLMs follow your habits too.

## Formulator (based on Data Formulator)

[Microsoft Data Formulator](https://github.com/microsoft/data-formulator) is MIT-licensed. The Formulator adopts its UX:

- **Data list**: model tables, columns and measures, with a data preview.
- **Concept encoding shelf**: x, y, color, size and column/row facets, plus the chart type. You can drag fields onto
  the shelf or type field names. If you type a name that doesn't exist yet, such as *Profit ratio*, VizardCraft derives it:
  - With an LLM, the model writes the DAX as a measure, or as a calculated column for row-level values.
  - Without one, rules handle ratio, per, margin, average, max/min, difference, running total, share of total and count.
  - Each derived field is validated by building and evaluating it. Failures are shown and the field isn't used.
- **Data threads**: every step (prompt + encoding → derived data → chart) is a node. Follow-up prompts branch from the
  selected node, which you can use to explore alternatives. Each step shows its derived data and its fields with their DAX.
- **Add to report** pins a step's chart to the current page and adds its derived fields to the model.
- **Compose report** builds a new page from the steps you picked. If you picked none, it uses the selected thread's path.
- The chart type is automatic until you choose one. The personal brain picks it. If a decision backend is configured,
  the decision model refines the choice and its probabilities are shown.

## Decisions and OpenJEV

`decide::Decide` answers typed questions with a probability for every option:

- **choice**: pick one option
- **noul**: yes or no
- **score**: ordered levels

VizardCraft uses decisions for fast routing, such as which chart type fits, whether a column is an ID, a date or a category,
and which transform comes next. There are three backends:

1. **Personal brain (local, default)**: Dirichlet priors from your past choices plus lexical evidence. No network.
2. **OpenJEV**: `POST {helper}/v1/systemone` with `{"model":"openjev","state":…,"questions":{name:{"type":"choice","instructions":…,"criteria":{…}}}}`.
   VizardCraft parses `answers.{name}.probabilities`, as well as `choice`, `confidence`, `noul` and `score`.
   It sends `Authorization: Bearer <SHIM_TOKEN>` when a token is set. If OpenJEV fails, VizardCraft falls back to the brain.
3. **Active chat model**: emulates typed decisions through a constrained JSON answer.

### Setting up OpenJEV

[OpenJEV](https://huggingface.co/openjev/openjev) is an open-weights 27B decision model. It answers in one forward pass per
question, handles up to 52 options and returns calibrated probabilities. Its method is described
[here](https://zefan-cai.github.io/open-jev/story/#method).

On an NVIDIA GPU with 80 GB of memory, the measured setup is FP8 with vLLM:

```bash
pip install "vllm==0.29.0" "openai==3.16.2" "httpx==0.28.1"
hf download openjev/openjev --local-dir openjev
vllm serve ./openjev --host 127.0.0.1 --served-model-name qwen --port 8000 \
  --enable-prefix-caching --max-model-len 16384 --gpu-memory-utilization 0.90 \
  --limit-mm-per-prompt '{"image":1}' --trust-remote-code --max-num-seqs 256 \
  --max-logprobs 64 --gdn-prefill-backend triton --quantization fp8
VLLM=http://localhost:8000/v1 TOKENIZER=./openjev READOUT_T=0.85 READOUT_NOUL_T=1.829074 \
READOUT_NOUL_BIAS=0 READOUT_TARGETED=1 READOUT_INSTR_STYLE=pyrepr SHIM_STAGGER=1 \
python openjev/helper/shim.py --host 127.0.0.1 --port 3000
```

Other builds:

- **FP8 checkpoint**: `openjev/openjev-FP8`, about 29 GB.
- **Apple silicon (MLX, text only)**: `openjev/openjev-MLX-4bit` is about 15 GB and `openjev/openjev-MLX` (8-bit) about 27 GB.
  Serve them with the repository's `helper/shim_mlx.py`.
- **llama.cpp (GGUF, text only)**: `openjev/openjev-GGUF`. Q4_K_M is 16.5 GB and fits a 24 GB GPU.
- **Smaller research variants**: `ZefanCai/Open-Jev-2B` and `ZefanCai/Open-Jev-9B` add a LoRA and a decision head over Qwen.

**Licence:** the OpenJEV weights are **CC BY-NC 4.0**. They are free for research and other non-commercial use, with attribution.
Commercial use needs a licence from the authors (support@loopai.com). The helper and serving files are Apache 2.0.
VizardCraft doesn't ship the weights. It only connects to a server you run.

Keep the helper on loopback. If you expose it to other machines, set `SHIM_TOKEN` and enter the token in the AI hub.

## The personal brain

Each installed copy of VizardCraft learns its user's habits, the way an ad account learns from every campaign.

**What is recorded.** Recording is opt-in, on by default, and disclosed in a banner the first time.
Events are appended to `~/.vizardcraft/brain/events.jsonl` and include:

- visuals created, with the visual type and the *types* of the fields used (date, measure, category, geo, ID, bool, plus cardinality)
- visual types you changed to
- fields added and filters
- formatting you set by hand (the key and value; free-text titles are recorded only as "custom")
- themes
- measures (DAX pattern and format string)
- Power Query steps per source kind, and sources used
- questions and what resolved them
- suggestions you accepted or dismissed
- typed decisions
- the hour of day

**Learners.** All learners train online, in Rust, on the device. Their weights are saved in `~/.vizardcraft/brain/model.json`.
If `model.json` is missing, it is rebuilt by replaying the log.

| Learner | Method | Used for |
|---|---|---|
| Chart recommender | Contextual multinomial logistic regression (SGD) over field-type features, initialised with visualization best practice and blended with Dirichlet counts per exact context | chart type chips, the Formulator's automatic chart, offline Copilot |
| Formatting preferences | Value counts per key and scope, with exponential decay | "Your style: Data labels on" |
| Transform sequences | Trigram → bigram → unigram interpolated n-gram per source kind | next Power Query step |
| DAX patterns | Pattern classes: year-over-year, period-to-date, ratio, share of total, ranking, filtered, iterator… | measure templates such as "Sales YoY %" |
| Question memory | BM25, with optional cosine similarity over provider embeddings | recalling past answers |
| Suggestion bandit | Thompson sampling, Beta(1 + accepts, 1 + dismissals) per suggestion family | which kinds of suggestion appear first |

`Brain::suggest(context)` returns scored suggestions, each with an explanation.
`Brain::profile_summary()` describes your habits for LLM prompts. `Brain::decider()` gives the local decision backend.

**Controls in AI hub > Personal brain:**

- turn learning on or off
- see what was learned: chart habits, formatting habits, when you work, DAX patterns, sources and suggestion acceptance
- **Export activity log**
- **Fine-tune export**
- **Forget everything**, which deletes the log and the model

### Fine-tuning: an optional MLOps path

**Fine-tune export** writes three files:

- `sft.jsonl`: chat-format instruction examples, such as which chart this user picks for certain fields,
  how they set a format key, their DAX measures, their next transform and questions they accepted.
- `decisions.jsonl`: typed decision examples with state, question, options and answer, in OpenJEV's style.
- `finetune_lora.py`: a script that LoRA-tunes a small model with Hugging Face transformers and peft.

```bash
pip install "transformers>=4.46" "peft>=0.13" "datasets>=3" accelerate
python finetune_lora.py --data sft.jsonl --base Qwen/Qwen2.5-0.5B-Instruct --out vizardcraft-me
python finetune_lora.py --merge --base Qwen/Qwen2.5-0.5B-Instruct --out vizardcraft-me
# convert to GGUF with llama.cpp, then: ollama create vizardcraft-me -f Modelfile
```

To use the result, pick provider **Ollama** and model `vizardcraft-me`. `decisions.jsonl` can also train a
decision head in the style of Open-Jev-2B.

## Privacy

- **Local models** (Ollama, LM Studio, llama.cpp, vLLM, OpenJEV) keep everything on your machine.
- **Cloud providers** receive your question, the model schema and the tool results the agent asks for.
  - **Send row values to cloud models** is on by default. Turn it off and tools return only schema, row counts and aggregates.
  - **Max rows a tool returns to the model** limits how many rows are sent.
- **The personal brain** never leaves your device.

## Hooks for other modules

The modules that own measures and Power Query call these (from `crate::ai`):

- `track_measure(app, name, dax, format)`, when a measure is created or edited
- `track_transform(app, source_kind, history_step_types, step_type)`, when a Power Query step is added
- `track_source(app, kind)`, when a data source is connected

The report module already calls `track_insert`, `track_field_added`, `track_kind_changed`, `track_format`,
`track_filter` and `track_theme`.

## Demo and screenshot hooks

```bash
VIZARDCRAFT_AI="copilot;ask=Show total sales by region" vizardcraft --sample --view report
VIZARDCRAFT_AI="seed;formulate=Total sales by region|enc:y=Profit ratio,x=Store[Category]" vizardcraft --sample
VIZARDCRAFT_AI="traces" …   # opens the trace viewer
```

The `seed` hook teaches an *in-memory* brain a few habits. It never writes to your real brain.
