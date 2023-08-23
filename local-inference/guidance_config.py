import guidance

mname = "meta-llama/Llama-2-7b-chat-hf"
guidance.llm = guidance.llms.Transformers(mname, device="mps", caching=False, temperature=0.5)
