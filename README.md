# Humanify
> Un-minify Javascript code using ChatGPT

This tool uses ChatGPT and other tools to un-minify Javascript code. Note that
ChatGPT does not perform any structural changes – it only provides hints to
rename variables and functions. The heavy lifting is done by Babel on AST level.

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

This tool does not yet work with long JS files. Code is not split, so be vary of
the ChatGPT's 16k context window. This roughly translates to 8k words in the
input file.

## Getting started

First install the dependencies:

```shell
npm install
```

Next up you'll need a ChatGPT API key. You can get one by signing up at
https://openai.com/.

There are several ways to provide the API key to the tool:
```shell
echo "OPENAI_TOKEN=your-token" > .env && npm start -- minified-file.js
export OPENAI_TOKEN="your-token" && npm start -- minified-file.js
OPENAI_TOKEN=your-token npm start -- minified-file.js
npm start -- --key="your-token" minified-file.js
```

Use your preferred way to provide the API key. Use `npm start -- --help` to see
all available options.

## Features

The main features of the tool are:
* Uses ChatGPT functions to get smart suggestions to rename variable and
  function names
* Uses custom and off-the-shelf Babel plugins to perform AST-level unmanging

## Contributing

If you'd like to contribute, please fork the repository and use a feature
branch. Pull requests are warmly welcome.

## Licensing

The code in this project is licensed under MIT license.
