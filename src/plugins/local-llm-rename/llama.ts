import {
  getLlama,
  LlamaChatSession,
  LlamaGrammar,
  LlamaModelOptions
} from "node-llama-cpp";
import { Gbnf } from "./gbnf.js";
import { getModelPath, getModelWrapper } from "../../local-models.js";
import { verbose } from "../../verbose.js";

export type Prompt = (
  systemPrompt: string,
  userPrompt: string,
  responseGrammar: Gbnf
) => Promise<string>;

const IS_CI = process.env["CI"] === "true";

export async function llama(opts: {
  seed?: number;
  model: string;
  disableGpu?: boolean;
}): Promise<Prompt> {
  const disableGpu = opts.disableGpu ?? IS_CI;
  const llama = await getLlama({ gpu: disableGpu ? false : "auto" });
  const modelOpts: LlamaModelOptions = {
    modelPath: getModelPath(opts?.model),
    gpuLayers: disableGpu ? 0 : undefined
  };
  verbose.log("Loading model with options", modelOpts);
  const model = await llama.loadModel(modelOpts);

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
