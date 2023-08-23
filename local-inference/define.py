import outlines.text as text
from guidance_config import guidance

@text.prompt
def prompt(system_prompt, user_message, assistant_message):
    """
    <s>[INST] <<SYS>>
    {{ system_prompt }}
    <</SYS>>

    {{ user_message }} [/INST]
    {{ assistant_message }}"""

def define(code):
  p = prompt(
     """Simplify the code snippet's purpose into a concise explanation in one sentence. Don't use variable names or function names in your description. Use the present tense.""",
     code,
     """{{gen "description" temperature=1 stop="."}}"""
  )
  result = guidance(p)()
  return result["description"]

def desc_to_name(description):
    p = prompt(
      """Create a name for a Javascript file for a code with the following description. Use lisp-case naming convention.""",
      description,
      """Sure, a good name for your Javascript file would be "{{gen "jsfilename" temperature=1 stop="."}}"""
    )
    result = guidance(p)()
    return result["jsfilename"] + ".js"