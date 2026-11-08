"""Create RANDOMLY-INITIALISED LoRA GGUF adapters for Qwen2.5-1.5B (swap-latency test only; not trained)."""
import numpy as np, gguf, sys
H, KV, L, R = 1536, 256, 28, 8
for name, seed, r in [("a", 1, 8), ("b", 2, 8), ("c", 3, 16)]:
    rng = np.random.default_rng(seed)
    w = gguf.GGUFWriter(f"adapters/lora_{name}.gguf", "qwen2")
    w.add_string("general.type", "adapter")
    w.add_string("adapter.type", "lora")
    w.add_float32("adapter.lora.alpha", float(r * 2))
    for i in range(L):
        for t, out in (("attn_q", H), ("attn_v", KV)):
            w.add_tensor(f"blk.{i}.{t}.weight.lora_a", (rng.standard_normal((r, H)) * 0.02).astype(np.float32))
            w.add_tensor(f"blk.{i}.{t}.weight.lora_b", (rng.standard_normal((out, r)) * 0.02).astype(np.float32))
    w.write_header_to_file(); w.write_kv_data_to_file(); w.write_tensors_to_file(); w.close()
