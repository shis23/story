#!/usr/bin/env python3
"""Patch gen/android/app/build.gradle.kts to add a release signingConfig.

GitHub Actions 用：`cargo tauri android init` 之后调用本脚本。gen/android 是
gitignore 的、每次 CI 重新生成，所以 signingConfig 不能写进版本库，必须 init
之后注入。signingConfig 从 gen/android/key.properties 读取（该文件由 workflow
从 GitHub Secret 生成）。

幂等：若已存在 signingConfig 则跳过。

用法（仓库根目录）：
    python3 scripts/ci-patch-android-signing.py
"""
from __future__ import annotations

import pathlib
import sys

GRADLE = pathlib.Path("crates/tauri-app/gen/android/app/build.gradle.kts")

# 注入到 android { } 块开头：读 key.properties，存在则建 release signingConfig。
SIGNING_BLOCK = """    // CI: release signing config (reads gen/android/key.properties)
    val sfKeyProps = Properties().apply {
        val kf = rootProject.file("key.properties")
        if (kf.exists()) kf.inputStream().use { load(it) }
    }
    if (sfKeyProps.containsKey("storeFile")) {
        signingConfigs {
            create("release") {
                storeFile = rootProject.file(sfKeyProps.getProperty("storeFile"))
                storePassword = sfKeyProps.getProperty("storePassword")
                keyAlias = sfKeyProps.getProperty("keyAlias")
                keyPassword = sfKeyProps.getProperty("keyPassword")
            }
        }
    }
"""


def main() -> int:
    if not GRADLE.exists():
        print(f"patch: {GRADLE} not found (run cargo tauri android init first)", file=sys.stderr)
        return 1

    s = GRADLE.read_text(encoding="utf-8")

    if 'create("release")' in s and "signingConfigs" in s:
        print("patch: signingConfig already present, skipping")
        return 0

    anchor = "android {\n"
    if anchor not in s:
        print("patch: 'android {' block not found", file=sys.stderr)
        return 1
    s = s.replace(anchor, anchor + SIGNING_BLOCK, 1)

    # 在 release buildType 里引用 signingConfig（仅当 key.properties 存在时）。
    needle = 'getByName("release") {\n            isMinifyEnabled = true'
    if needle not in s:
        print("patch: release buildType needle not found (tauri template changed?)", file=sys.stderr)
        return 1
    replacement = (
        'getByName("release") {\n'
        '            if (sfKeyProps.containsKey("storeFile")) signingConfig = signingConfigs.getByName("release")\n'
        '            isMinifyEnabled = true'
    )
    s = s.replace(needle, replacement, 1)

    GRADLE.write_text(s, encoding="utf-8")
    print(f"patch: signingConfig injected into {GRADLE}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
