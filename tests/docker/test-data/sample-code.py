#!/usr/bin/env python3
"""
Sample Python code for testing
"""

def calculate_sum(numbers):
    """Calculate sum of a list of numbers"""
    return sum(numbers)

def find_max(numbers):
    """Find maximum number in a list"""
    if not numbers:
        return None
    return max(numbers)

def process_file(filename):
    """Process a text file and count lines"""
    try:
        with open(filename, 'r') as f:
            lines = f.readlines()
        return len(lines)
    except FileNotFoundError:
        return -1

if __name__ == "__main__":
    test_numbers = [1, 5, 3, 9, 2]
    print(f"Sum: {calculate_sum(test_numbers)}")
    print(f"Max: {find_max(test_numbers)}")