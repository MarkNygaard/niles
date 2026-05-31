# niles-recognition

Speaker-embedding extraction via ECAPA-TDNN ONNX models. Used by niles to
identify *who* is speaking once enrollment + matching land in a follow-up PR.

## Model

Recommended model: `speechbrain/spkrec-ecapa-voxceleb` on HuggingFace. To run,
export to ONNX. The resulting `.onnx` file is ~30 MB and lives outside the
repo. Point `[recognition].model_path` at the file.

## Testing

Set `NILES_ECAPA_MODEL_PATH=/abs/path/to/model.onnx` to enable inference-level
tests; without it, those tests skip and print a
`(NILES_ECAPA_MODEL_PATH not set; skipping)` notice. Pure-math helpers always
run.

## Scope

v1 is inference only. Enrollment, matching, per-speaker storage, and Wyoming
pipeline integration are follow-up PRs. Do NOT extend this crate's scope until
the follow-up plan is on paper.
