import sys,time,ctypes,llama_cpp as L
t=time.time(); llm=L.Llama(model_path=sys.argv[1],n_ctx=512,n_threads=4,verbose=False,seed=0); print("load",time.time()-t,flush=True)
def gen():
    llm.reset(); t=time.time(); llm.create_completion("Plan: open the budget sheet.",max_tokens=8,temperature=0); return time.time()-t
print("gen0",gen(),gen(),flush=True)
t=time.time(); a=L.llama_adapter_lora_init(llm._model.model,b"adapters/lora_a.gguf"); print("init",time.time()-t,flush=True)
arr=(L.llama_adapter_lora_p_ctypes*1)(a); sc=(ctypes.c_float*1)(1.0)
t=time.time(); print(L.llama_set_adapters_lora(llm._ctx.ctx,arr,1,sc), "set",time.time()-t,flush=True)
print("gen_lora",gen(),gen(),flush=True)
