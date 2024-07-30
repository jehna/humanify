import {
  GemmaChatWrapper,
  getLlama,
  LlamaChatSession,
  LlamaGrammar,
} from "node-llama-cpp";

export async function llama(seed?: number) {
  const llama = await getLlama();
  const model = await llama.loadModel({
    modelPath: "models/model.gguf",
  });

  const context = await model.createContext({ seed });

  return async (
    systemPrompt: string,
    userPrompt: string,
    responseGrammar: string
  ) => {
    const session = new LlamaChatSession({
      contextSequence: context.getSequence(),
      autoDisposeSequence: true,
      chatWrapper: new GemmaChatWrapper(),
      systemPrompt,
    });
    const response = await session.promptWithMeta(userPrompt, {
      temperature: 0.8,
      grammar: new LlamaGrammar(llama, {
        grammar: responseGrammar,
      }),
      stopOnAbortSignal: true,
    });
    session.dispose();
    return response.responseText;
  };
}

function grammarString(grammar: (string | RegExp)[]) {
  return (
    "root ::= " +
    grammar
      .map((rule) =>
        rule instanceof RegExp
          ? rule.source
          : `"${rule.replaceAll('"', '\\"')}"`
      )
      .join(" ")
  );
}
