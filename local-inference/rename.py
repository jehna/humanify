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

def rename(code_before, code_after, var_name, filename):
  p2 = prompt(
    """Your task is to read the code in file "{}" and write the purpose of each variable in one sentence.""".format(filename),
    code_before + code_after,
    '"' + var_name + '"' + """ is {{gen "vardesc" temperature=1 stop="\n"}}"""
  )

  result3 = guidance(p2)()
  print(result3['vardesc'])

  p3 = prompt(
      """You are a Code Assistant.""",
      """What would be a good name for the following function or a variable?.\n""" + result3['vardesc'],
      """A good name would be "{{gen "varname" temperature=1 stop_regex="[^a-zA-Z0-9]"}}"""
  )


  result4 = guidance(p3)()
  return result4['varname']
