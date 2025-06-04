#!/usr/bin/env python3
# Test validation script to verify sandbox environment

import os
import sys
import subprocess
import json

def run_command(cmd):
    # Run a command and return output
    try:
        result = subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=10)
        return result.returncode, result.stdout, result.stderr
    except subprocess.TimeoutExpired:
        return -1, "", "Command timed out"

def validate_environment():
    # Validate the sandbox environment is working correctly
    tests = [
        ("ls /workspace", "Directory listing"),
        ("cat /workspace/test-files/config.txt", "File reading"),
        ("echo 'test' > /workspace/temp/test.txt", "File writing"),
        ("python3 --version", "Python availability"),
        ("node --version", "Node.js availability"),
        ("git --version", "Git availability"),
    ]
    
    results = []
    for cmd, desc in tests:
        code, stdout, stderr = run_command(cmd)
        results.append({
            "test": desc,
            "command": cmd,
            "exit_code": code,
            "success": code == 0,
            "output": stdout.strip() if stdout else stderr.strip()
        })
    
    return results

if __name__ == "__main__":
    results = validate_environment()
    
    print("Sandbox Environment Validation Results:")
    print("=" * 50)
    
    for result in results:
        status = "✓ PASS" if result["success"] else "✗ FAIL"
        print(f"{status} {result['test']}")
        if not result["success"]:
            print(f"    Command: {result['command']}")
            print(f"    Output: {result['output']}")
    
    # Return exit code based on success
    failed_tests = [r for r in results if not r["success"]]
    if failed_tests:
        print(f"\n{len(failed_tests)} tests failed!")
        sys.exit(1)
    else:
        print(f"\nAll {len(results)} tests passed!")
        sys.exit(0)