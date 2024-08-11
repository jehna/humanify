# HumanifyJS
> Deobfuscate Javascript code using LLMs ("AI")

This tool uses large language modeles (like ChatGPT & llama) and other tools to
deobfuscate, unminify, transpile, decompile and unpack Javascript code. Note
that LLMs don't perform any structural changes – they only provide hints to
rename variables and functions. The heavy lifting is done by Babel on AST level
to ensure code stays 1-1 equivalent.

### Version 2 is out! 🎉

Highlights:
* Python not required anymore!
* A lot of tests, the codebase is actually maintanable now
* Renewed CLI tool `humanify` installable via npm

### ➡️ Check out the [introduction blog post][blogpost] for in-depth explanation!

[blogpost]: https://thejunkland.com/blog/using-llms-to-reverse-javascript-minification

## Example

Given the following minified code:

```javascript
function a(e,t){var n=[];var r=e.length;var i=0;for(;i<r;i+=t){if(i+t<r){n.push(e.substring(i,i+t))}else{n.push(e.substring(i,r))}}return n}
```

The tool will output a human-readable version:

```javascript
function splitString(inputString, chunkSize) {
  var chunks = [];
  var stringLength = inputString.length;
  var startIndex = 0;
  for (; startIndex < stringLength; startIndex += chunkSize) {
    if (startIndex + chunkSize < stringLength) {
      chunks.push(inputString.substring(startIndex, startIndex + chunkSize));
    } else {
      chunks.push(inputString.substring(startIndex, stringLength));
    }
  }
  return chunks;
}
```

🚨 **NOTE:** 🚨

Large files may take some time to process and use a lot of tokens if you use
ChatGPT. For a rough estimate, the tool takes about 2 tokens per character to
process a file:

```shell
echo "$((2 * $(wc -c < yourscript.min.js)))"
```

So for refrence: a minified `bootstrap.min.js` would take about $0.5 to
un-minify using ChatGPT.

Using `humanify local` is of course free, but may take more time, be less
accurate and not possible with your existing hardware.

## Getting started

### Installation

The fastest way to try out the tool is to run it using `npx`:

```
npx humanifyjs
```

This will download the tool and run it locally. If you want to install the tool
globally, you can do so with:

```shell
npm install -g humanifyjs
```

After the installation is done, you should be able to run the tool globally:

```shell
humanify
```

### Usage

Next you'll need to decide whether to use `openai` or `local` mode. In a
nutshell:

* `openai` mode
  * Runs on someone else's computer that's specifically optimized for this kind
    of things
  * Costs money depending on the length of your code
  * Is more accurate
* `local` mode
  * Runs locally
  * Is free
  * Is less accurate
  * Needs a local GPU with ~8gb RAM (M1 Mac works just fine)
  * Runs as fast as your GPU does

See instructions below for each option:

### OpenAI mode

You'll need a ChatGPT API key. You can get one by signing up at
https://openai.com/.

There are several ways to provide the API key to the tool:
```shell
echo "OPENAI_API_KEY=your-token" > .env && humanify openai obfuscated-file.js
export OPENAI_API_KEY="your-token" && humanify openai obfuscated-file.js
OPENAI_TOKEN=your-token humanify openai obfuscated-file.js
humanify --apiKey="your-token" obfuscated-file.js
```

Use your preferred way to provide the API key. Use `humanify --help` to see
all available options.

### Local mode

The local mode uses a pre-trained language model to deobfuscate the code. The
model is not included in the repository due to its size, but you can download it
using the following command:

```shell
humanify download 2gb
```

This downloads the `2gb` model to your local machine. This is only needed to do
once. You can also choose to download other models depending on your local
resources. List the available models using `humanify download`.

After downloading the model, you can run the tool with:

```shell
humanify local obfuscated-file.js
```

This uses your local GPU to deobfuscate the code. If you don't have a GPU, the
tool will automatically fall back to CPU mode. Note that using a GPU speeds up
the process significantly.

Humanify has native support for Apple's M-series chips, and can fully utilize
the GPU capabilities of your Mac.

## Features

The main features of the tool are:
* Uses ChatGPT functions/local models to get smart suggestions to rename
  variable and function names
* Uses custom and off-the-shelf Babel plugins to perform AST-level unmanging
* Uses Webcrack to unbundle Webpack bundles

## Contributing

If you'd like to contribute, please fork the repository and use a feature
branch. Pull requests are warmly welcome.

## Licensing

The code in this project is licensed under MIT license.
