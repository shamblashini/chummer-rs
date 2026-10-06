# Checking files in real Chummer5a

chummer-rs writes the `.chum5` format of Chummer5a 5.226. To check that a
file loads in Chummer itself, build Chummer5a from source and run it under
Wine with real .NET Framework 4.8.

## Build (Linux, .NET SDK 10)

In a copy of the Chummer5a repository:

1. Move `global.json` out of the way. It pins SDK 8.0.401.
2. In `Chummer/Chummer.csproj`, add
   `<PackageReference Include="System.Resources.Extensions" Version="10.0.0" />`.
   Without it the resx compile fails (MSB3823).
3. The project file spells one custom data folder with a different case
   than the folder on disk. Link it:
   `ln -s "Exclude German sourcebooks" "Chummer/customdata/Exclude German Sourcebooks"`.
4. Build:

   ```bash
   dotnet build Chummer/Chummer.csproj -c Release -p:EnableWindowsTargeting=true \
     -p:GenerateResourceUsePreserializedResources=true -p:PreBuildEvent= \
     -p:PostBuildEvent= -p:RunAnalyzers=false
   ```

The program is `Chummer/bin/Release/Chummer5.exe`.

## Run

```bash
cd Chummer/bin/Release
WINEPREFIX=~/.local/share/wineprefixes/chummer WINEDEBUG=-all wine ./Chummer5.exe 'Z:\path\to\file.chum5'
```

The file argument must be a Windows path (`Z:` maps to `/`).

## Result, 2026-10-04

Characters made with `chummer-cli new` load with no dialogs or warnings:

- A mundane Human (Priority DEABC).
- An Elf Magician with free Spellcasting and Summoning.

Chummer's Karma Summary shows the same budgets as chummer-rs. Re-saving in Chummer drops only `<chummerrsversion>`. Chummer adds:

- Its usual calculated values.
- An "Unarmed Attack" weapon, which every Chummer character has.

## Compressed saves (.chum5lz), 2026-10-06

Checked with a Chummer5a 5.225 release build under the same Wine setup:

- `chummer-cli new` wrote a character, Chummer opened it and saved it with
  File > Save As as "Compressed Chummer5 Saves". That file is
  `crates/chummer-core/tests/chum5lz/fixer-chummer.chum5lz`. Its header is
  `5D 00 00 00 01` + eight `FF` bytes (lc 3, lp 0, pb 2, 16 MiB
  dictionary, unknown size); `xz --format=lzma -d` and chummer-rs decode it
  to the same XML.
- `chummer-cli new out.chum5lz` wrote a compressed character. It passes
  `xz --format=lzma -t`, and Chummer's File > Open loaded it with only the
  usual "created in a later version" question that 5.225 asks for any
  5.226 file.

To drive Chummer from a script on a hidden X display, focus its window
first (`xdotool windowfocus`); keyboard input does not reach it otherwise.
The Save As dialog appends the filter's extension, so type the name
without `.chum5lz` or rename the result.
