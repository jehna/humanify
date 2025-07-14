# HumanifyJS Windows Installation & Troubleshooting Guide

HumanifyJS is a powerful tool, but its dependency on a native Node.js addon, `isolated-vm`, can lead to installation challenges on Windows. This addon requires a C++ compiler and specific development tools to be present on your system.

This guide provides a step-by-step process to set up the necessary environment and troubleshoot common errors.

## 1. Prerequisites

Before attempting to install HumanifyJS, you **must** install the following prerequisites.

### A. Visual Studio Build Tools

This is the most critical step. `isolated-vm` needs to be compiled from source, and this requires the Microsoft C++ toolchain.

1.  Download the **Visual Studio Build Tools** from the [official Microsoft website](https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022).
2.  Run the installer.
3.  In the "Workloads" tab, select **"Desktop development with C++"**.
4.  In the "Installation details" pane on the right, ensure that the **"MSVC..."** and **"Windows ... SDK"** components are selected. The defaults are usually sufficient.
5.  Proceed with the installation.

> **PowerShell Automation:**
> You can also install this non-interactively using an administrator PowerShell terminal:
> ```powershell
> winget install Microsoft.VisualStudio.BuildTools --force --override "--wait --quiet --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
> ```

### B. Node.js (via a Version Manager)

Compatibility issues often arise from using an incorrect version of Node.js. We strongly recommend using a Node.js version manager to easily install and switch to a known-compatible version.

1.  **Install a Node Version Manager.** We recommend `nvs` (Node Version Switcher). You can install it with Scoop:
    ```bash
    scoop install nvs
    ```
2.  **Install the latest LTS (Long-Term Support) version of Node.js.**
    ```bash
    nvs add lts
    nvs use lts
    ```
3.  **IMPORTANT: Open a new terminal window.** This is crucial for the `nvs use` command to take effect and for your PATH to be updated correctly.
4.  Verify the correct version is active:
    ```bash
    node -v
    ```

## 2. Installing HumanifyJS

Once all prerequisites are in place, open a **new terminal** and run the global installation command:

```bash
npm install -g humanifyjs
```

If the installation succeeds, you're all set! You can verify it by running `humanify --version`.

## 3. Troubleshooting Common Errors

If the installation fails, here are the most common errors and their solutions.

### Error: `MSBUILD : error MSB1009: Project file does not exist.` or C++ compilation errors

This error almost always means the Visual Studio Build Tools are missing, not installed correctly, or not accessible in your terminal's environment.

*   **Solution:**
    1.  Re-run the Visual Studio Installer and ensure the **"Desktop development with C++"** workload is checked and installed.
    2.  Make sure you have opened a **new terminal** *after* the installation was complete.
    3.  Try running the installation from a "Developer Command Prompt for VS" which can be found in your Start Menu. This guarantees all build tools are in the PATH.

### Error: `node-gyp rebuild failed`

This is a generic error that can have several causes.

*   **Solutions:**
    1.  **Check Prerequisites:** Double-check that you have completed all steps in the "Prerequisites" section above.
    2.  **Clean npm Cache:** A corrupted package in the cache can cause persistent failures. Force-clean the cache and try again:
        ```bash
        npm cache clean --force
        ```
    3.  **Check for Python:** `node-gyp` sometimes requires Python. While the C++ workload often handles this, if errors persist, install Python from the Microsoft Store or python.org and ensure it's in your PATH.

By following this guide, you should be able to resolve the common installation issues for HumanifyJS on a Windows system.