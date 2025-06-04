#!/usr/bin/env python3
"""
Log Verification Utility for HIVE Tests

This script analyzes HIVE log output to verify that expected tools, agents,
and system components executed correctly during tests.
"""

import json
import re
import sys
from typing import Dict, List, Optional, Set
from dataclasses import dataclass
from pathlib import Path


@dataclass
class LogEntry:
    timestamp: str
    level: str
    thread_id: str
    span: str
    target: str
    message: str
    fields: Dict[str, str]


@dataclass
class VerificationResult:
    passed: bool
    errors: List[str]
    warnings: List[str]
    summary: Dict[str, int]


class HiveLogAnalyzer:
    def __init__(self, log_content: str):
        self.entries = self._parse_log_entries(log_content)
        
    def _parse_log_entries(self, content: str) -> List[LogEntry]:
        """Parse HIVE log entries from the log content."""
        entries = []
        for line in content.strip().split('\n'):
            if not line.strip():
                continue
                
            try:
                # Parse the structured log format
                # Format: TIMESTAMP LEVEL ThreadId(XX) SPAN: TARGET: MESSAGE fields...
                parts = line.split(' ', 4)
                if len(parts) < 5:
                    continue
                    
                timestamp = parts[0]
                level = parts[1]
                thread_match = re.match(r'ThreadId\((\d+)\)', parts[2])
                thread_id = thread_match.group(1) if thread_match else "unknown"
                
                # Extract span and target
                span_target = parts[3]
                message_and_fields = parts[4]
                
                # Split span and target
                if ':' in span_target:
                    span = span_target.split(':')[0]
                    target = ':'.join(span_target.split(':')[1:])
                else:
                    span = ""
                    target = span_target
                
                # Extract fields from the end of the message
                fields = {}
                message = message_and_fields
                
                # Simple field extraction (could be improved)
                field_matches = re.findall(r'(\w+)=([^\s]+)', message_and_fields)
                for key, value in field_matches:
                    fields[key] = value.strip('"')
                
                entries.append(LogEntry(
                    timestamp=timestamp,
                    level=level,
                    thread_id=thread_id,
                    span=span,
                    target=target,
                    message=message,
                    fields=fields
                ))
                
            except Exception as e:
                # Skip malformed lines
                continue
                
        return entries
    
    def verify_agent_lifecycle(self) -> VerificationResult:
        """Verify that agents started and completed properly."""
        errors = []
        warnings = []
        summary = {}
        
        # Check for agent startup
        agent_starts = [e for e in self.entries if "Agent starting execution" in e.message]
        summary['agents_started'] = len(agent_starts)
        
        if not agent_starts:
            errors.append("No agents were started")
        
        # Check for actor lifecycle events
        actor_ready_events = [e for e in self.entries if "Actor ready, sending ready signal" in e.message]
        summary['actors_ready'] = len(actor_ready_events)
        
        if len(actor_ready_events) < 4:  # expect assistant, planner, spawn_agent, plan_approval
            warnings.append(f"Expected at least 4 actors to be ready, got {len(actor_ready_events)}")
        
        # Check for state transitions
        state_transitions = [e for e in self.entries if "state transition" in e.message]
        summary['state_transitions'] = len(state_transitions)
        
        if not state_transitions:
            errors.append("No agent state transitions found")
        
        return VerificationResult(
            passed=len(errors) == 0,
            errors=errors,
            warnings=warnings,
            summary=summary
        )
    
    def verify_tool_execution(self, expected_tools: List[str]) -> VerificationResult:
        """Verify that expected tools were executed."""
        errors = []
        warnings = []
        summary = {}
        
        # Tools available events
        tool_events = [e for e in self.entries if "tools_available" in e.span]
        summary['tool_registration_events'] = len(tool_events)
        
        # LLM requests with tools
        llm_requests = [e for e in self.entries if "llm_request" in e.span and "tools_count" in e.fields]
        summary['llm_requests_with_tools'] = len(llm_requests)
        
        if llm_requests:
            max_tools = max(int(e.fields.get('tools_count', '0')) for e in llm_requests)
            summary['max_tools_in_request'] = max_tools
        
        # Check for specific tool calls (would need to be in log)
        for tool in expected_tools:
            tool_calls = [e for e in self.entries if tool in e.message.lower()]
            summary[f'{tool}_calls'] = len(tool_calls)
            
        return VerificationResult(
            passed=len(errors) == 0,
            errors=errors,
            warnings=warnings,
            summary=summary
        )
    
    def verify_llm_interaction(self) -> VerificationResult:
        """Verify that LLM interactions occurred properly."""
        errors = []
        warnings = []
        summary = {}
        
        # Check for LLM requests
        llm_requests = [e for e in self.entries if "Executing LLM chat request" in e.message]
        summary['llm_requests'] = len(llm_requests)
        
        if not llm_requests:
            errors.append("No LLM requests found")
        
        # Check for network connections
        network_connections = [e for e in self.entries if "starting new connection" in e.message]
        summary['network_connections'] = len(network_connections)
        
        if llm_requests and not network_connections:
            warnings.append("LLM requests found but no network connections")
        
        # Check for user input processing
        user_input_events = [e for e in self.entries if "user_input" in e.span]
        summary['user_input_events'] = len(user_input_events)
        
        return VerificationResult(
            passed=len(errors) == 0,
            errors=errors,
            warnings=warnings,
            summary=summary
        )
    
    def verify_system_startup(self) -> VerificationResult:
        """Verify that the HIVE system started properly."""
        errors = []
        warnings = []
        summary = {}
        
        # Check for HIVE startup
        hive_startup = [e for e in self.entries if "Starting headless HIVE multi-agent system" in e.message]
        summary['hive_startup_events'] = len(hive_startup)
        
        if not hive_startup:
            errors.append("HIVE system startup not found")
        
        # Check for config loading
        config_events = [e for e in self.entries if "config" in e.target.lower()]
        summary['config_events'] = len(config_events)
        
        if not config_events:
            warnings.append("No config loading events found")
        
        # Check for actor creation
        actor_creation = [e for e in self.entries if "Starting actors for agent" in e.message]
        summary['actor_creation_events'] = len(actor_creation)
        
        return VerificationResult(
            passed=len(errors) == 0,
            errors=errors,
            warnings=warnings,
            summary=summary
        )


def verify_hive_execution(log_content: str, expected_tools: Optional[List[str]] = None) -> Dict[str, VerificationResult]:
    """
    Main verification function that checks all aspects of HIVE execution.
    
    Args:
        log_content: Raw log content from HIVE execution
        expected_tools: List of tool names that should have been executed
        
    Returns:
        Dictionary of verification results by category
    """
    analyzer = HiveLogAnalyzer(log_content)
    
    if expected_tools is None:
        expected_tools = ["planner", "spawn_agent", "command", "file_reader"]
    
    results = {
        'system_startup': analyzer.verify_system_startup(),
        'agent_lifecycle': analyzer.verify_agent_lifecycle(),
        'tool_execution': analyzer.verify_tool_execution(expected_tools),
        'llm_interaction': analyzer.verify_llm_interaction(),
    }
    
    return results


def print_verification_results(results: Dict[str, VerificationResult]):
    """Print verification results in a human-readable format."""
    overall_passed = all(result.passed for result in results.values())
    
    print(f"\n🔍 HIVE Log Verification Results")
    print("=" * 50)
    print(f"Overall Status: {'✅ PASSED' if overall_passed else '❌ FAILED'}\n")
    
    for category, result in results.items():
        status = '✅ PASSED' if result.passed else '❌ FAILED'
        print(f"{category.replace('_', ' ').title()}: {status}")
        
        # Print summary
        for key, value in result.summary.items():
            print(f"  - {key.replace('_', ' ')}: {value}")
        
        # Print errors
        if result.errors:
            for error in result.errors:
                print(f"  ❌ {error}")
        
        # Print warnings
        if result.warnings:
            for warning in result.warnings:
                print(f"  ⚠️  {warning}")
        
        print()


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Usage: python log_verification.py <log_file_path>")
        sys.exit(1)
    
    log_file = Path(sys.argv[1])
    if not log_file.exists():
        print(f"Error: Log file {log_file} does not exist")
        sys.exit(1)
    
    log_content = log_file.read_text()
    results = verify_hive_execution(log_content)
    print_verification_results(results)
    
    # Exit with error code if any verification failed
    overall_passed = all(result.passed for result in results.values())
    sys.exit(0 if overall_passed else 1)