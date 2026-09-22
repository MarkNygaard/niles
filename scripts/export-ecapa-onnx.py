#!/usr/bin/env python3
"""Export SpeechBrain's ECAPA-TDNN speaker encoder to ONNX.

Niles identifies who is speaking by comparing 192-float voice prints.
Producing them needs the model as ONNX, and SpeechBrain publishes
PyTorch weights — so somebody has to run this once. The result is about
30 MB and is the same file forever; it is not part of the repository
because a public repo is a poor place for a binary nothing diffs.

    pip install torch speechbrain onnx
    python scripts/export-ecapa-onnx.py -o ecapa.onnx

Then attach it to a GitHub release and point the image build at it, per
deploy/docker/README.md.

The exported graph takes raw 16 kHz mono audio as [1, samples] and
returns [1, 1, 192]. Niles reads the *last* dimension and requires it
to be 192, so the leading axes do not matter — but the count does, and
this script checks it rather than leaving it to be discovered when a
house stops recognising anybody.
"""

from __future__ import annotations

import argparse
import hashlib
import sys
from pathlib import Path

SOURCE = "speechbrain/spkrec-ecapa-voxceleb"
SAMPLE_RATE = 16_000
EMBEDDING_DIM = 192


def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "-o",
        "--output",
        type=Path,
        default=Path("ecapa.onnx"),
        help="where to write the ONNX file (default: ./ecapa.onnx)",
    )
    p.add_argument(
        "--opset",
        type=int,
        default=17,
        help="ONNX opset (default: 17)",
    )
    return p.parse_args()


def main() -> int:
    args = parse_args()

    try:
        import torch
        from speechbrain.inference.speaker import EncoderClassifier
    except ImportError as e:
        print(f"missing dependency: {e}", file=sys.stderr)
        print("try: pip install torch speechbrain onnx", file=sys.stderr)
        return 1

    print(f"fetching {SOURCE} …", file=sys.stderr)
    classifier = EncoderClassifier.from_hparams(source=SOURCE, savedir="/tmp/ecapa")

    class Encoder(torch.nn.Module):
        """Mel features and the encoder as one graph.

        Exported together on purpose. Splitting them would leave the
        feature extraction to be reimplemented on the Rust side, and
        two implementations of a mel filterbank drift — quietly, into
        voice prints that no longer match the ones already enrolled.
        """

        def __init__(self, inner):
            super().__init__()
            self.inner = inner

        def forward(self, wav: "torch.Tensor") -> "torch.Tensor":
            feats = self.inner.mods.compute_features(wav)
            feats = self.inner.mods.mean_var_norm(
                feats, torch.ones(wav.shape[0], device=wav.device)
            )
            return self.inner.mods.embedding_model(feats)

    model = Encoder(classifier).eval()

    # Three seconds, which is about what a satellite captures.
    dummy = torch.randn(1, SAMPLE_RATE * 3)

    with torch.no_grad():
        out = model(dummy)
    got = out.shape[-1]
    if got != EMBEDDING_DIM:
        print(
            f"refusing to export: embeddings are {got}-dim, niles needs {EMBEDDING_DIM}",
            file=sys.stderr,
        )
        return 1

    args.output.parent.mkdir(parents=True, exist_ok=True)
    print(f"exporting to {args.output} …", file=sys.stderr)
    torch.onnx.export(
        model,
        dummy,
        str(args.output),
        input_names=["wav"],
        output_names=["embedding"],
        # Utterances are not all three seconds long, and a fixed length
        # would silently truncate or pad every one that is not.
        dynamic_axes={"wav": {0: "batch", 1: "samples"}, "embedding": {0: "batch"}},
        opset_version=args.opset,
    )

    try:
        import onnx

        onnx.checker.check_model(str(args.output))
    except ImportError:
        print("onnx not installed; skipping the validity check", file=sys.stderr)

    data = args.output.read_bytes()
    print(f"\n{args.output}", file=sys.stderr)
    print(f"  size    {len(data) / 1_048_576:.1f} MiB", file=sys.stderr)
    print(f"  sha256  {hashlib.sha256(data).hexdigest()}", file=sys.stderr)
    print("\nAttach it to a release and pass the URL + sha256 to the image", file=sys.stderr)
    print("build as ECAPA_URL / ECAPA_SHA256. See deploy/docker/README.md.", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
