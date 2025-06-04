#!/bin/bash
# Intentionally broken script for testing error handling

echo "Starting broken script..."

# This will fail - file doesn't exist
cat /nonexistent/file.txt

# This will also fail - invalid command
invalid_command_that_does_not_exist

echo "This line should not be reached"