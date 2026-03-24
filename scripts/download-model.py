#!/usr/bin/env python3
"""
Download CodeRankEmbed model and export to ONNX.

Usage:
    pip install optimum[onnxruntime] sentence-transformers
    python scripts/download-model.py

Output:
    models/CodeRankEmbed/
        tokenizer.json          — BPE tokenizer for the Rust `tokenizers` crate
        config.json             — model config
        onnx/model.onnx         — full-precision ONNX model (~547 MB)
        onnx-quantized/         — int8 quantized model (~137 MB, preferred)
"""

import os
import sys
import shutil
from pathlib import Path

MODEL_ID = "nomic-ai/CodeRankEmbed"
BASE_DIR = Path(__file__).resolve().parent.parent / "models" / "CodeRankEmbed"
ONNX_DIR = BASE_DIR / "onnx"
QUANTIZED_DIR = BASE_DIR / "onnx-quantized"


def check_deps():
    """Verify required Python packages are installed."""
    missing = []
    try:
        import optimum  # noqa: F401
    except ImportError:
        missing.append("optimum[onnxruntime]")
    try:
        import sentence_transformers  # noqa: F401
    except ImportError:
        missing.append("sentence-transformers")
    try:
        import onnxruntime  # noqa: F401
    except ImportError:
        missing.append("onnxruntime")

    if missing:
        print(f"Missing packages: {', '.join(missing)}")
        print(f"Install with: pip install {' '.join(missing)}")
        sys.exit(1)


def download_tokenizer():
    """Download tokenizer.json and config files from HuggingFace."""
    from huggingface_hub import hf_hub_download

    print(f"Downloading tokenizer and config from {MODEL_ID}...")
    BASE_DIR.mkdir(parents=True, exist_ok=True)

    files_to_download = [
        "tokenizer.json",
        "tokenizer_config.json",
        "special_tokens_map.json",
        "vocab.txt",
        "config.json",
        "config_sentence_transformers.json",
    ]

    for filename in files_to_download:
        try:
            path = hf_hub_download(
                repo_id=MODEL_ID,
                filename=filename,
                local_dir=str(BASE_DIR),
            )
            print(f"  Downloaded: {filename}")
        except Exception as e:
            print(f"  Skipped {filename}: {e}")


def export_onnx():
    """Export the model to ONNX format using optimum-cli."""
    print(f"\nExporting to ONNX (full precision)...")
    ONNX_DIR.mkdir(parents=True, exist_ok=True)

    ret = os.system(
        f'optimum-cli export onnx '
        f'--model {MODEL_ID} '
        f'--task feature-extraction '
        f'--trust-remote-code '
        f'"{ONNX_DIR}"'
    )
    if ret != 0:
        print("ONNX export failed. Trying alternative approach...")
        export_onnx_manual()
    else:
        print(f"  Exported to: {ONNX_DIR / 'model.onnx'}")


def export_onnx_manual():
    """Fallback: export using optimum Python API directly."""
    try:
        from optimum.onnxruntime import ORTModelForFeatureExtraction

        print("  Using optimum Python API...")
        model = ORTModelForFeatureExtraction.from_pretrained(
            MODEL_ID, export=True, trust_remote_code=True
        )
        model.save_pretrained(str(ONNX_DIR))
        print(f"  Exported to: {ONNX_DIR / 'model.onnx'}")
    except Exception as e:
        print(f"  Manual export also failed: {e}")
        print("  You may need to export manually. See README.")
        sys.exit(1)


def quantize():
    """Quantize the ONNX model to int8 for smaller size and faster inference."""
    print(f"\nQuantizing to int8...")
    QUANTIZED_DIR.mkdir(parents=True, exist_ok=True)

    # Try optimum-cli first
    ret = os.system(
        f'optimum-cli onnxruntime quantize '
        f'--onnx_model "{ONNX_DIR}" '
        f'--avx512 '
        f'-o "{QUANTIZED_DIR}"'
    )

    if ret != 0:
        print("  CLI quantization failed, trying Python API...")
        try:
            from optimum.onnxruntime import ORTQuantizer
            from optimum.onnxruntime.configuration import AutoQuantizationConfig

            quantizer = ORTQuantizer.from_pretrained(str(ONNX_DIR))
            qconfig = AutoQuantizationConfig.avx512_vnni(is_static=False)
            quantizer.quantize(save_dir=str(QUANTIZED_DIR), quantization_config=qconfig)
            print(f"  Quantized to: {QUANTIZED_DIR}")
        except Exception as e:
            print(f"  Quantization failed: {e}")
            print("  Full-precision model is still available at onnx/model.onnx")
    else:
        print(f"  Quantized to: {QUANTIZED_DIR}")


def copy_tokenizer():
    """Copy tokenizer.json to the root model dir for easy access by Rust."""
    src = BASE_DIR / "tokenizer.json"
    if not src.exists():
        # Check if optimum put it in the onnx dir
        alt = ONNX_DIR / "tokenizer.json"
        if alt.exists():
            shutil.copy2(alt, src)
            print(f"\nCopied tokenizer.json to {BASE_DIR}")


def verify():
    """Verify the exported model works."""
    print("\nVerifying model...")
    try:
        import onnxruntime as ort
        from tokenizers import Tokenizer

        # Check tokenizer
        tok_path = BASE_DIR / "tokenizer.json"
        if not tok_path.exists():
            tok_path = ONNX_DIR / "tokenizer.json"
        tokenizer = Tokenizer.from_file(str(tok_path))
        encoding = tokenizer.encode("fn main() { println!(\"hello\"); }", add_special_tokens=True)
        print(f"  Tokenizer OK: {len(encoding.ids)} tokens")

        # Check ONNX model
        model_path = QUANTIZED_DIR / "model_quantized.onnx"
        if not model_path.exists():
            model_path = QUANTIZED_DIR / "model.onnx"
        if not model_path.exists():
            model_path = ONNX_DIR / "model.onnx"

        session = ort.InferenceSession(str(model_path))
        print(f"  ONNX model OK: {model_path.name}")
        print(f"  Inputs: {[i.name for i in session.get_inputs()]}")
        print(f"  Outputs: {[o.name for o in session.get_outputs()]}")

        # Quick inference test
        import numpy as np
        ids = np.array([encoding.ids[:512]], dtype=np.int64)
        mask = np.array([encoding.attention_mask[:512]], dtype=np.int64)
        outputs = session.run(None, {"input_ids": ids, "attention_mask": mask})
        print(f"  Output shape: {outputs[0].shape}")
        print(f"  Embedding dim: {outputs[0].shape[-1]}")

        print("\nModel ready for use with BearWisdom.")
        print(f"  Set model_dir to: {BASE_DIR}")
        print(f"  Or set BW_MODEL_DIR={BASE_DIR}")

    except Exception as e:
        print(f"  Verification failed: {e}")
        print("  The model files may still be usable — check paths manually.")


def main():
    print("=" * 60)
    print("CodeRankEmbed Model Download + ONNX Export")
    print("=" * 60)
    print(f"Model: {MODEL_ID}")
    print(f"Output: {BASE_DIR}")
    print()

    check_deps()
    download_tokenizer()
    export_onnx()
    quantize()
    copy_tokenizer()
    verify()

    print()
    print("=" * 60)
    print("Done. Model files:")
    for p in sorted(BASE_DIR.rglob("*")):
        if p.is_file():
            size_mb = p.stat().st_size / (1024 * 1024)
            print(f"  {p.relative_to(BASE_DIR)}  ({size_mb:.1f} MB)")
    print("=" * 60)


if __name__ == "__main__":
    main()
