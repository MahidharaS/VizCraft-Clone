//! The optional MLOps path: turn the brain's event log into training data for a small local
//! model, so the user's own LLM (served by Ollama / llama.cpp) learns their habits too.
//!
//! Two datasets:
//! - `sft.jsonl`: chat-format instruction tuning (`{"messages": [system, user, assistant]}`),
//!   for LoRA on e.g. Qwen2.5-0.5B/1.5B-Instruct with `finetune_lora.py`;
//! - `decisions.jsonl`: typed decisions (`{"state", "question", "options", "answer"}`), the format
//!   of OpenJEV-style decision models (e.g. the Open-Jev-2B LoRA + decision head research setup).

use serde_json::{Value, json};

use crate::brain::{Event, EventKind, FieldSig};
use crate::brain::learners::CHART_CLASSES;

const SYSTEM: &str = "You are the VizardCraft assistant of one specific user. Follow their habits.";

fn fields_text(fields: &[FieldSig]) -> String {
    fields.iter().map(|f| format!("{} ({})", f.name, f.ty)).collect::<Vec<_>>().join(", ")
}

fn chat(user: String, assistant: String) -> Value {
    json!({"messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": user}, {"role": "assistant", "content": assistant}]})
}

/// Instruction-tuning examples from events.
pub fn sft_examples(events: &[Event]) -> Vec<Value> {
    let mut out = Vec::new();
    for e in events {
        match &e.kind {
            EventKind::VisualCreated { kind, fields, source } if !fields.is_empty() && source != "copilot" => {
                out.push(chat(format!("Which visual type should I use for these fields: {}?", fields_text(fields)), kind.clone()));
            }
            EventKind::VisualKindChanged { to, fields, .. } if !fields.is_empty() => {
                out.push(chat(format!("Which visual type should I use for these fields: {}?", fields_text(fields)), to.clone()));
            }
            EventKind::Format { scope, key, value } => out.push(chat(format!("How do I usually set `{key}` on a {scope} visual?"), value.clone())),
            EventKind::Theme { name } => out.push(chat("Which report theme do I prefer?".into(), name.clone())),
            EventKind::Measure { name, dax, format } => out.push(chat(format!("Write the DAX measure `{name}`."), if format.is_empty() { dax.clone() } else { format!("{dax}\n-- format: {format}") })),
            EventKind::Transform { source, history, step } => {
                out.push(chat(format!("Applied Power Query steps on {source} data so far: [{}]. What is my next step?", history.join(", ")), step.clone()));
            }
            EventKind::Question { text, resolution, accepted: true } => out.push(chat(text.clone(), resolution.clone())),
            _ => {}
        }
    }
    out
}

/// Decision-model examples (state + typed question + the option the user chose).
pub fn decision_examples(events: &[Event]) -> Vec<Value> {
    let mut out = Vec::new();
    for e in events {
        if let EventKind::VisualCreated { kind, fields, .. } | EventKind::VisualKindChanged { to: kind, fields, .. } = &e.kind
            && !fields.is_empty()
        {
            out.push(json!({"state": format!("Fields: {}", fields_text(fields)), "question": "Which chart type does this user pick?", "type": "choice", "options": CHART_CLASSES, "answer": kind}));
        }
        if let EventKind::Suggestion { arm, id, accepted } = &e.kind {
            out.push(json!({"state": format!("Suggestion family: {arm}. Suggestion: {id}"), "question": "Will the user accept this suggestion?", "type": "noul", "answer": accepted}));
        }
        if let EventKind::Decision { key, choice } = &e.kind {
            out.push(json!({"state": "", "question": key, "type": "choice", "answer": choice}));
        }
    }
    out
}

pub const LORA_SCRIPT: &str = r#"#!/usr/bin/env python3
"""LoRA-tune a small open model on your VizardCraft habits (exported by VizardCraft: AI > Personal brain > Fine-tune export).

    pip install "transformers>=4.46" "peft>=0.13" "datasets>=3" accelerate
    python finetune_lora.py --data sft.jsonl --base Qwen/Qwen2.5-0.5B-Instruct --out vizardcraft-me

Then merge and serve locally, e.g. with Ollama:
    python finetune_lora.py --merge --base Qwen/Qwen2.5-0.5B-Instruct --out vizardcraft-me
    # convert with llama.cpp:  python convert_hf_to_gguf.py vizardcraft-me-merged --outfile vizardcraft-me.gguf
    # Modelfile:  FROM ./vizardcraft-me.gguf
    ollama create vizardcraft-me -f Modelfile
and pick provider "Ollama", model "vizardcraft-me" in VizardCraft's AI settings.

Decision models: decisions.jsonl holds typed choices (state, question, options, answer). It can
train a decision head in the style of Open-Jev-2B / -9B (LoRA over Qwen with a single-position
readout of option letters); see https://zefan-cai.github.io/open-jev/story/#method.
Note: OpenJEV's released weights are CC BY-NC 4.0 (non-commercial).
"""
import argparse, json

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", default="sft.jsonl")
    ap.add_argument("--base", default="Qwen/Qwen2.5-0.5B-Instruct")
    ap.add_argument("--out", default="vizardcraft-me")
    ap.add_argument("--epochs", type=float, default=3)
    ap.add_argument("--lr", type=float, default=2e-4)
    ap.add_argument("--rank", type=int, default=16)
    ap.add_argument("--merge", action="store_true")
    a = ap.parse_args()

    from transformers import AutoTokenizer, AutoModelForCausalLM, TrainingArguments, Trainer, DataCollatorForLanguageModeling
    from peft import LoraConfig, get_peft_model, PeftModel
    from datasets import Dataset

    tok = AutoTokenizer.from_pretrained(a.base)
    if tok.pad_token is None:
        tok.pad_token = tok.eos_token

    if a.merge:
        model = AutoModelForCausalLM.from_pretrained(a.base)
        model = PeftModel.from_pretrained(model, a.out).merge_and_unload()
        model.save_pretrained(a.out + "-merged"); tok.save_pretrained(a.out + "-merged")
        print("merged ->", a.out + "-merged"); return

    rows = [json.loads(l) for l in open(a.data, encoding="utf-8") if l.strip()]
    texts = [tok.apply_chat_template(r["messages"], tokenize=False) for r in rows]
    ds = Dataset.from_dict({"text": texts}).map(lambda b: tok(b["text"], truncation=True, max_length=1024), batched=True, remove_columns=["text"])

    model = AutoModelForCausalLM.from_pretrained(a.base)
    model = get_peft_model(model, LoraConfig(r=a.rank, lora_alpha=2 * a.rank, lora_dropout=0.05, target_modules=["q_proj", "k_proj", "v_proj", "o_proj"], task_type="CAUSAL_LM"))
    model.print_trainable_parameters()
    args = TrainingArguments(output_dir=a.out, num_train_epochs=a.epochs, learning_rate=a.lr, per_device_train_batch_size=4, gradient_accumulation_steps=4, logging_steps=10, save_strategy="epoch", report_to=[])
    Trainer(model=model, args=args, train_dataset=ds, data_collator=DataCollatorForLanguageModeling(tok, mlm=False)).train()
    model.save_pretrained(a.out); tok.save_pretrained(a.out)
    print("LoRA adapter ->", a.out)

if __name__ == "__main__":
    main()
"#;

/// Writes `sft.jsonl`, `decisions.jsonl` and `finetune_lora.py` into `dir`. Returns the counts.
pub fn export(events: &[Event], dir: &std::path::Path) -> Result<(usize, usize), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let sft = sft_examples(events);
    let dec = decision_examples(events);
    let lines = |v: &[Value]| v.iter().map(|x| x.to_string() + "\n").collect::<String>();
    std::fs::write(dir.join("sft.jsonl"), lines(&sft)).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("decisions.jsonl"), lines(&dec)).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("finetune_lora.py"), LORA_SCRIPT).map_err(|e| e.to_string())?;
    Ok((sft.len(), dec.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_datasets() {
        let ev = vec![
            Event::now(EventKind::VisualCreated { kind: "line".into(), fields: vec![FieldSig::new("date", "D"), FieldSig::new("measure", "S")], source: "gallery".into() }),
            Event::now(EventKind::Measure { name: "Margin".into(), dax: "DIVIDE([P], [S])".into(), format: "0%".into() }),
            Event::now(EventKind::Suggestion { arm: "chart".into(), id: "chart:line".into(), accepted: true }),
        ];
        let dir = std::env::temp_dir().join(format!("vizardcraft-ft-{}", std::process::id()));
        let (a, b) = export(&ev, &dir).unwrap();
        assert_eq!((a, b), (2, 2));
        let first: Value = serde_json::from_str(std::fs::read_to_string(dir.join("sft.jsonl")).unwrap().lines().next().unwrap()).unwrap();
        assert_eq!(first["messages"][2]["content"], "line");
        assert!(std::fs::read_to_string(dir.join("finetune_lora.py")).unwrap().contains("peft"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
