#!/usr/bin/env python3
"""Train and export the VCP Mobile Chinese affect classifier.

The script intentionally pins both the backbone and dataset revisions so the
model package can be reproduced and audited. Training artifacts are written to
the requested output directory; only the final INT8 ONNX package needs to be
copied into the Android plugin assets.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import random
from collections import Counter
from pathlib import Path

import numpy as np
import onnxruntime as ort
import torch
from datasets import load_dataset
from opencc import OpenCC
from onnxruntime.quantization import QuantType, quantize_dynamic
from sklearn.metrics import accuracy_score, classification_report, f1_score
from sklearn.model_selection import train_test_split
from torch.utils.data import DataLoader, Dataset
from transformers import (
    AutoModelForSequenceClassification,
    AutoTokenizer,
    get_linear_schedule_with_warmup,
)

BACKBONE = "hfl/rbt3"
BACKBONE_REVISION = "0aa0527ff4170f29e1dfd3eb6ef60dc67e1bf75c"
DATASET = "Johnson8187/Chinese_Multi-Emotion_Dialogue_Dataset"
DATASET_REVISION = "119d246a1595b44fd2cdccff0d9b288eafee25d1"
MODEL_ID = "vcp/rbt3-zh-dialogue-affect"
MODEL_VERSION = "1.0.0-int8"

LABELS = [
    "neutral",
    "joy",
    "sadness",
    "anger",
    "confusion",
    "disgust",
    "surprise",
    "affection",
]

RAW_LABELS = {
    "平淡語氣": "neutral",
    "開心語調": "joy",
    "悲傷語調": "sadness",
    "憤怒語調": "anger",
    "疑問語調": "confusion",
    "厭惡語調": "disgust",
    "驚奇語調": "surprise",
    "關切語調": "affection",
}

SAMPLE_TEXTS = [
    "今天终于把这个问题解决了，我真的很开心。",
    "我有点难过，感觉最近什么都做不好。",
    "你为什么一直不理我？我现在很生气。",
    "这到底是什么意思，我有点搞不懂。",
    "这个味道太恶心了，我完全受不了。",
    "真的吗？你居然已经完成了！",
    "你还好吗？记得照顾好自己。",
    "今天晚上吃什么？",
]

# Product-domain examples are appended only after the frozen validation/test
# split is created. They improve relationship-chat phrasing without leaking
# into the reported holdout metrics.
HARD_TRAINING_EXAMPLES = [
    ("今天就正常聊聊天吧。", "neutral"),
    ("我先去忙一会儿，晚点再说。", "neutral"),
    ("晚上吃什么？", "neutral"),
    ("你现在有空吗？", "neutral"),
    ("我刚到家。", "neutral"),
    ("先按原来的计划继续。", "neutral"),
    ("太好了，你终于回我了！", "joy"),
    ("今天真的特别开心。", "joy"),
    ("成功了，我就知道我们可以。", "joy"),
    ("看到你我心情一下变好了。", "joy"),
    ("这真是个好消息。", "joy"),
    ("我现在高兴得想笑。", "joy"),
    ("你都不理我，我真的很难过。", "sadness"),
    ("我感觉自己被丢下了。", "sadness"),
    ("今天心里空空的，什么都不想做。", "sadness"),
    ("我有点失落，可能是我期待太多了。", "sadness"),
    ("为什么最后还是变成这样，我好难受。", "sadness"),
    ("我忍不住想哭。", "sadness"),
    ("你为什么一直不理我？我现在很生气。", "anger"),
    ("你到底有没有在听，我真的火了。", "anger"),
    ("别再敷衍我了，我很愤怒。", "anger"),
    ("为什么每次都这样？气死我了。", "anger"),
    ("我不是困惑，我就是在生气。", "anger"),
    ("你这样做让我特别恼火。", "anger"),
    ("这到底是什么意思，我有点搞不懂。", "confusion"),
    ("我没看懂你刚才在说什么。", "confusion"),
    ("等等，前后好像对不上。", "confusion"),
    ("我现在有点迷糊，能解释一下吗？", "confusion"),
    ("为什么会得出这个结论？我不理解。", "confusion"),
    ("这句话到底应该怎么理解？", "confusion"),
    ("这个味道太恶心了，我完全受不了。", "disgust"),
    ("这种做法让我觉得很反感。", "disgust"),
    ("看见这个我就觉得恶心。", "disgust"),
    ("这也太令人厌恶了。", "disgust"),
    ("我完全无法接受这种肮脏的行为。", "disgust"),
    ("别把这么恶心的东西发给我。", "disgust"),
    ("真的吗？你居然已经完成了！", "surprise"),
    ("什么？这结果完全出乎我的意料。", "surprise"),
    ("天啊，怎么会突然变成这样！", "surprise"),
    ("你竟然还记得，我太意外了。", "surprise"),
    ("没想到你会主动来找我。", "surprise"),
    ("这也太突然了吧！", "surprise"),
    ("你还好吗？记得照顾好自己。", "affection"),
    ("我很在意你，也会一直陪着你。", "affection"),
    ("我爱你，看到你就会安心。", "affection"),
    ("别太累了，我会担心你。", "affection"),
    ("不管发生什么，我都站在你这边。", "affection"),
    ("我只是想抱抱你。", "affection"),
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--epochs", type=int, default=8)
    parser.add_argument("--batch-size", type=int, default=32)
    parser.add_argument("--learning-rate", type=float, default=2e-5)
    parser.add_argument("--max-length", type=int, default=128)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--patience", type=int, default=2)
    return parser.parse_args()


def set_seed(seed: int) -> None:
    random.seed(seed)
    np.random.seed(seed)
    torch.manual_seed(seed)
    torch.cuda.manual_seed_all(seed)
    torch.backends.cudnn.deterministic = True
    torch.backends.cudnn.benchmark = False


class AffectDataset(Dataset):
    def __init__(self, texts: list[str], labels: list[int], tokenizer, max_length: int):
        self.encodings = tokenizer(
            texts,
            max_length=max_length,
            padding="max_length",
            truncation=True,
            return_tensors="pt",
        )
        self.labels = torch.tensor(labels, dtype=torch.long)

    def __len__(self) -> int:
        return len(self.labels)

    def __getitem__(self, index: int) -> dict[str, torch.Tensor]:
        item = {key: value[index] for key, value in self.encodings.items()}
        item["labels"] = self.labels[index]
        return item


def evaluate(model, loader: DataLoader, device: torch.device) -> tuple[dict, list[int], list[int]]:
    model.eval()
    predictions: list[int] = []
    expected: list[int] = []
    losses: list[float] = []
    with torch.inference_mode():
        for batch in loader:
            labels = batch.pop("labels").to(device)
            inputs = {key: value.to(device) for key, value in batch.items()}
            output = model(**inputs, labels=labels)
            losses.append(float(output.loss.detach().cpu()))
            predictions.extend(output.logits.argmax(dim=-1).detach().cpu().tolist())
            expected.extend(labels.detach().cpu().tolist())
    metrics = {
        "loss": float(np.mean(losses)) if losses else 0.0,
        "accuracy": accuracy_score(expected, predictions),
        "macroF1": f1_score(expected, predictions, average="macro"),
        "weightedF1": f1_score(expected, predictions, average="weighted"),
    }
    return metrics, expected, predictions


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def export_onnx(model, tokenizer, output_dir: Path, max_length: int) -> tuple[Path, Path]:
    model = model.to("cpu").eval()
    dummy = tokenizer(
        "今天心情很好",
        max_length=max_length,
        padding="max_length",
        truncation=True,
        return_tensors="pt",
    )
    input_names = [name for name in ("input_ids", "attention_mask", "token_type_ids") if name in dummy]
    args = tuple(dummy[name] for name in input_names)
    dynamic_axes = {name: {0: "batch", 1: "sequence"} for name in input_names}
    dynamic_axes["logits"] = {0: "batch"}
    fp32_path = output_dir / "model.fp32.onnx"
    torch.onnx.export(
        model,
        args,
        fp32_path,
        input_names=input_names,
        output_names=["logits"],
        dynamic_axes=dynamic_axes,
        opset_version=17,
        do_constant_folding=True,
        dynamo=False,
    )
    int8_path = output_dir / "model.onnx"
    quantize_dynamic(
        fp32_path,
        int8_path,
        per_channel=True,
        reduce_range=False,
        weight_type=QuantType.QInt8,
    )
    return fp32_path, int8_path


def predict_onnx(session: ort.InferenceSession, tokenizer, texts: list[str], max_length: int) -> list[dict]:
    encoded = tokenizer(
        texts,
        max_length=max_length,
        padding="max_length",
        truncation=True,
        return_tensors="np",
    )
    input_names = {item.name for item in session.get_inputs()}
    inputs = {name: value.astype(np.int64) for name, value in encoded.items() if name in input_names}
    logits = session.run([session.get_outputs()[0].name], inputs)[0]
    logits = logits - logits.max(axis=-1, keepdims=True)
    probabilities = np.exp(logits) / np.exp(logits).sum(axis=-1, keepdims=True)
    results = []
    for text, scores in zip(texts, probabilities, strict=True):
        order = np.argsort(scores)[::-1]
        results.append(
            {
                "text": text,
                "top": LABELS[int(order[0])],
                "score": float(scores[order[0]]),
                "margin": float(scores[order[0]] - scores[order[1]]),
                "scores": {label: float(scores[index]) for index, label in enumerate(LABELS)},
            }
        )
    return results


def main() -> None:
    args = parse_args()
    set_seed(args.seed)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    device = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    print(f"device={device} torch={torch.__version__}", flush=True)

    raw = load_dataset(DATASET, revision=DATASET_REVISION)["train"]
    converter = OpenCC("t2s")
    texts: list[str] = []
    labels: list[int] = []
    for row in raw:
        canonical = RAW_LABELS.get(str(row["emotion"]).strip())
        if canonical is None:
            continue
        text = converter.convert(str(row["text"]).strip())
        if text:
            texts.append(text)
            labels.append(LABELS.index(canonical))
    print("label_counts=", Counter(LABELS[label] for label in labels), flush=True)

    train_texts, holdout_texts, train_labels, holdout_labels = train_test_split(
        texts,
        labels,
        test_size=0.30,
        random_state=args.seed,
        stratify=labels,
    )
    validation_texts, test_texts, validation_labels, test_labels = train_test_split(
        holdout_texts,
        holdout_labels,
        test_size=0.50,
        random_state=args.seed,
        stratify=holdout_labels,
    )
    for text, label in HARD_TRAINING_EXAMPLES:
        train_texts.append(text)
        train_labels.append(LABELS.index(label))

    tokenizer = AutoTokenizer.from_pretrained(BACKBONE, revision=BACKBONE_REVISION)
    id2label = {index: label for index, label in enumerate(LABELS)}
    label2id = {label: index for index, label in id2label.items()}
    model = AutoModelForSequenceClassification.from_pretrained(
        BACKBONE,
        revision=BACKBONE_REVISION,
        num_labels=len(LABELS),
        id2label=id2label,
        label2id=label2id,
        ignore_mismatched_sizes=True,
    ).to(device)

    train_dataset = AffectDataset(train_texts, train_labels, tokenizer, args.max_length)
    validation_dataset = AffectDataset(validation_texts, validation_labels, tokenizer, args.max_length)
    test_dataset = AffectDataset(test_texts, test_labels, tokenizer, args.max_length)
    generator = torch.Generator().manual_seed(args.seed)
    train_loader = DataLoader(
        train_dataset,
        batch_size=args.batch_size,
        shuffle=True,
        generator=generator,
        pin_memory=device.type == "cuda",
    )
    validation_loader = DataLoader(validation_dataset, batch_size=args.batch_size * 2)
    test_loader = DataLoader(test_dataset, batch_size=args.batch_size * 2)

    counts = np.bincount(train_labels, minlength=len(LABELS)).astype(np.float32)
    class_weights = torch.tensor(counts.sum() / (len(LABELS) * counts), device=device)
    optimizer = torch.optim.AdamW(model.parameters(), lr=args.learning_rate, weight_decay=0.01)
    total_steps = len(train_loader) * args.epochs
    scheduler = get_linear_schedule_with_warmup(
        optimizer,
        num_warmup_steps=max(1, int(total_steps * 0.10)),
        num_training_steps=total_steps,
    )
    scaler = torch.amp.GradScaler("cuda", enabled=device.type == "cuda")
    best_macro_f1 = -math.inf
    stale_epochs = 0
    best_path = args.output_dir / "best_state.pt"
    history = []

    for epoch in range(1, args.epochs + 1):
        model.train()
        running_loss = 0.0
        for batch in train_loader:
            labels_tensor = batch.pop("labels").to(device)
            inputs = {key: value.to(device, non_blocking=True) for key, value in batch.items()}
            optimizer.zero_grad(set_to_none=True)
            with torch.amp.autocast("cuda", enabled=device.type == "cuda"):
                logits = model(**inputs).logits
                loss = torch.nn.functional.cross_entropy(logits, labels_tensor, weight=class_weights)
            scaler.scale(loss).backward()
            scaler.unscale_(optimizer)
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            scaler.step(optimizer)
            scaler.update()
            scheduler.step()
            running_loss += float(loss.detach().cpu())

        validation_metrics, _, _ = evaluate(model, validation_loader, device)
        epoch_record = {
            "epoch": epoch,
            "trainLoss": running_loss / max(1, len(train_loader)),
            **validation_metrics,
        }
        history.append(epoch_record)
        print(json.dumps(epoch_record, ensure_ascii=False), flush=True)
        if validation_metrics["macroF1"] > best_macro_f1 + 1e-5:
            best_macro_f1 = validation_metrics["macroF1"]
            stale_epochs = 0
            torch.save(model.state_dict(), best_path)
        else:
            stale_epochs += 1
            if stale_epochs >= args.patience:
                break

    model.load_state_dict(torch.load(best_path, map_location=device, weights_only=True))
    test_metrics, expected, predictions = evaluate(model, test_loader, device)
    report = classification_report(
        expected,
        predictions,
        labels=list(range(len(LABELS))),
        target_names=LABELS,
        output_dict=True,
        zero_division=0,
    )
    metrics = {
        "backbone": BACKBONE,
        "backboneRevision": BACKBONE_REVISION,
        "dataset": DATASET,
        "datasetRevision": DATASET_REVISION,
        "seed": args.seed,
        "split": {
            "train": len(train_dataset),
            "validation": len(validation_dataset),
            "test": len(test_dataset),
        },
        "history": history,
        "test": test_metrics,
        "classificationReport": report,
    }
    (args.output_dir / "metrics.json").write_text(
        json.dumps(metrics, ensure_ascii=False, indent=2), encoding="utf-8"
    )

    tokenizer.save_pretrained(args.output_dir / "tokenizer")
    fp32_path, int8_path = export_onnx(model, tokenizer, args.output_dir, args.max_length)
    digest = sha256(int8_path)
    manifest = {
        "modelId": MODEL_ID,
        "modelVersion": MODEL_VERSION,
        "labels": LABELS,
        "inputIdsName": "input_ids",
        "attentionMaskName": "attention_mask",
        "tokenTypeIdsName": "token_type_ids",
        "outputName": "logits",
        "lowerCase": True,
        "maxLength": args.max_length,
        "sha256": digest,
        "backbone": BACKBONE,
        "backboneRevision": BACKBONE_REVISION,
        "dataset": DATASET,
        "datasetRevision": DATASET_REVISION,
    }
    (args.output_dir / "model.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8"
    )

    session = ort.InferenceSession(str(int8_path), providers=["CPUExecutionProvider"])
    predictions_json = predict_onnx(session, tokenizer, SAMPLE_TEXTS, args.max_length)
    (args.output_dir / "sample_predictions.json").write_text(
        json.dumps(predictions_json, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    print(
        json.dumps(
            {
                "test": test_metrics,
                "fp32Bytes": fp32_path.stat().st_size,
                "int8Bytes": int8_path.stat().st_size,
                "sha256": digest,
                "samples": predictions_json,
            },
            ensure_ascii=False,
            indent=2,
        ),
        flush=True,
    )


if __name__ == "__main__":
    main()
