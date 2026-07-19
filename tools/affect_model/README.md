# VCP Mobile affect classifier

This directory contains the reproducible training and ONNX export pipeline for
the Android on-device affect classifier.

The model is based on `hfl/rbt3` at commit
`0aa0527ff4170f29e1dfd3eb6ef60dc67e1bf75c` and is fine-tuned on
`Johnson8187/Chinese_Multi-Emotion_Dialogue_Dataset` at commit
`119d246a1595b44fd2cdccff0d9b288eafee25d1`.

Run:

```powershell
python tools/affect_model/train_export.py --output-dir tmp/affect-model-output
```

The final Android package consists of `model.onnx`, `model.json`, and the
tokenizer's `vocab.txt`. Training checkpoints and the FP32 ONNX file are not
shipped in the application.
