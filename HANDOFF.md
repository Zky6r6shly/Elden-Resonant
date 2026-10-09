# Where Elden Resonant stands

For a Claude session running on the PC that has Elden Ring and CONTROL Resonant installed.

- **Melty draft:** "Elden Resonant", mod id `cf72f13d-c096-4afb-88dc-5c6b78a544f0`, Studio https://melty.gg/studio/cf72f13d-c096-4afb-88dc-5c6b78a544f0. License: all rights reserved, remixing off.
- **Release 0.1.0** is submitted as a draft. Melty's one-click check says yes; Melty still wants one play through Melty before it publishes.
- **Recipe:** installs `modengine2/` from `EldenResonant-0.1.0.zip` into `{managed}/modengine2` (needs the ModEngine2 loader and Elden Ring 2.7.1.0–2.7.1.1; CONTROL Resonant is a secondary game). Built by `tools/package.sh`.

## Still to do (needs this PC)

1. In the Melty app, open Elden Resonant from Studio and press **Play**. Confirm Elden Ring starts and `%LOCALAPPDATA%\EldenResonant\elden-resonant.log` shows "params applied", text passes with strings replaced, and the CONTROL Resonant sounds it picked.
2. Make a **Resonant** character and check:
   - all six Aberrant forms and their skills
   - the seven Resonance powers (they should cost FP and never run out)
   - Double Jump, Elevate and Rush on the jump button
   - F8 for free powers
   - the sounds
3. Tune `sheets/traversal.json` and fix anything broken. Change the sheet first, then run `tools/package.sh` and resubmit.
4. Take a gameplay screenshot that shows only the game, then add it to the listing with add_screenshot and finish_screenshot.
5. Show the creator a summary, and publish only when they say so.
