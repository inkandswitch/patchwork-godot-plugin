import os
import subprocess

# open plugin.cfg, replace the line with `version=<version>` with `version=<git describe --tags --abbrev=6>`
with open("plugin.cfg", "r") as file:
    lines = file.readlines()

git_describe = subprocess.check_output(["git", "describe", "--tags", "--abbrev=6"]).decode("utf-8").strip()
print(git_describe)

# if it has more than two `-` in the version, replace all the subsequent `-` with `+`
if git_describe.count("-") >= 2:
    first_index = git_describe.find("-")
    if first_index != -1:
        git_describe = git_describe[:first_index] + "-" + git_describe[first_index + 1 :].replace("-", "+")
        print(git_describe)

new_lines: list[str] = []
for line in lines:
    if line.startswith("version="):
        line = 'version="' + git_describe + '"\n'
    new_lines.append(line)

with open("plugin.cfg", "w") as file:
    file.writelines(new_lines)
