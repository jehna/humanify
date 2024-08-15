import { getLlama, LlamaChatSession, LlamaGrammar } from "node-llama-cpp";
import { Gbnf } from "./gbnf.js";
import { getModelPath, getModelWrapper } from "../../local-models.js";

export type Prompt = (
  systemPrompt: string,
  userPrompt: string,
  responseGrammar: Gbnf
) => Promise<string>;

const IS_CI = process.env["CI"] === "true";

export async function llama(opts: {
  seed?: number;
  model: string;
  disableGPU?: boolean;
}): Promise<Prompt> {
  const llama = await getLlama();
  const model = await llama.loadModel({
    modelPath: getModelPath(opts?.model),
    gpuLayers: (opts?.disableGPU ?? IS_CI) ? 0 : undefined
  });

  const context = await model.createContext({ seed: opts?.seed });

  return async (systemPrompt, userPrompt, responseGrammar) => {
    const session = new LlamaChatSession({
      contextSequence: context.getSequence(),
      autoDisposeSequence: true,
      systemPrompt,
      chatWrapper: getModelWrapper(opts.model)
    });
    const response = await session.promptWithMeta(userPrompt, {
      temperature: 0.8,
      grammar: new LlamaGrammar(llama, {
        grammar: `${responseGrammar}`
      }),
      stopOnAbortSignal: true
    });
    session.dispose();
    return responseGrammar.parseResult(response.responseText);
  };
}
