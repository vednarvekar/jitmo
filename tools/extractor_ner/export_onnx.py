"""Export a fine-tuned token classifier to ONNX, quantise to int8, and write what the Rust loader needs.

    python -I export_onnx.py --model models_tmp/minilm_l6 --out ../../models/ner

Writes model.onnx (fp32), model_int8.onnx, tokenizer.json, labels.json into --out.
Inputs: input_ids, attention_mask, token_type_ids (int64, [1, n]); output: logits [1, n, num_labels].
"""
import argparse
import json
import shutil
import sys
from pathlib import Path

import torch
from onnxruntime.quantization import QuantType, quantize_dynamic
from transformers import AutoModelForTokenClassification, AutoTokenizer


class Wrap(torch.nn.Module):
    def __init__(self, m):
        super().__init__()
        self.m = m

    def forward(self, input_ids, attention_mask, token_type_ids):
        return self.m(input_ids=input_ids, attention_mask=attention_mask, token_type_ids=token_type_ids).logits


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--opset", type=int, default=17)
    a = ap.parse_args()
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    model = AutoModelForTokenClassification.from_pretrained(a.model).eval()
    tok = AutoTokenizer.from_pretrained(a.model)
    ids = torch.tensor([tok("remind me to call mom at 6pm")["input_ids"]])
    mask = torch.ones_like(ids)
    types = torch.zeros_like(ids)
    fp32 = out / "model.onnx"
    torch.onnx.export(
        Wrap(model), (ids, mask, types), str(fp32),
        input_names=["input_ids", "attention_mask", "token_type_ids"], output_names=["logits"],
        dynamic_axes={k: {0: "b", 1: "n"} for k in ["input_ids", "attention_mask", "token_type_ids", "logits"]},
        opset_version=a.opset, dynamo=False,
    )
    quantize_dynamic(str(fp32), str(out / "model_int8.onnx"), weight_type=QuantType.QInt8)
    # tokenizer.json for the Rust `tokenizers` crate
    tj = Path(a.model) / "tokenizer.json"
    if tj.exists():
        shutil.copy(tj, out / "tokenizer.json")
    else:
        tok.backend_tokenizer.save(str(out / "tokenizer.json"))
    labels = [model.config.id2label[i] for i in range(model.config.num_labels)]
    (out / "labels.json").write_text(json.dumps(labels), encoding="utf-8")
    print("wrote", [p.name + f" {p.stat().st_size / 1e6:.1f}MB" for p in sorted(out.iterdir())])


if __name__ == "__main__":
    sys.exit(main())
