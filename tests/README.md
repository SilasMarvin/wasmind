# Spawn Agent Integration Tests

This directory contains comprehensive integration tests for the spawn agent functionality, which tests the entire multi-agent orchestration system of the HIVE architecture.

## Test Scenario Matrix

### **Basic Wait/No-Wait Patterns** (`basic_scenarios.rs`)
1. **No Wait + Immediate Complete** - Spawn, continue immediately, agent completes in background
2. **Wait + Immediate Complete** - Spawn, wait, agent completes quickly, parent resumes  
3. **No Wait + Long Running** - Spawn, continue, agent takes time to complete
4. **Wait + Long Running** - Spawn, wait, agent works for extended period, parent waits properly

### **Plan Approval Workflows** (`plan_approval.rs`) 
Child agent uses planner tool requiring manager approval:

5. **Wait + Child Planner + Manager Approves** - Parent waits → Child needs approval → Parent approves → Child continues → Child completes → Parent resumes
6. **Wait + Child Planner + Manager Rejects** - Parent waits → Child needs approval → Parent rejects → Child handles rejection → Parent resumes
7. **No Wait + Child Planner + Async Approval** - Parent continues → Child needs approval → Parent approves later
8. **Wait + Child Planner + Manager Timeout** - What happens if manager never responds?

### **Information Request Workflows** (`info_request.rs`)
Child agent requests additional information from parent:

9. **Wait + Child Requests Info** - Parent waits → Child requests info → Parent provides → Child continues
10. **No Wait + Child Info Request** - Parent continues → Child requests info → Parent responds asynchronously
11. **Wait + Multiple Info Requests** - Child requests info multiple times during execution

### **Multiple Agent Scenarios** (`multiple_agents.rs`)
Testing coordination of multiple spawned agents:

12. **Multiple Agents + Wait All** - Spawn 3 agents, wait for all to complete
13. **Multiple Agents + No Wait** - Spawn 3 agents, continue immediately  
14. **Mixed Agent Types** - Spawn mix of Workers and SubManagers
15. **Sequential Completion** - Multiple agents complete at different times
16. **Parallel Tool Usage** - Multiple spawned agents using tools simultaneously

### **Nested Spawning (Multi-Level)** (`nested_spawning.rs`)
Testing hierarchical agent spawning:

17. **Child Spawns Grandchild** - Agent → spawns SubManager → spawns Worker
18. **Deep Nesting with Wait** - Complex wait chains through multiple levels
19. **Sibling Spawning** - Multiple children each spawn their own agents
20. **Cross-Level Communication** - Grandchild needs approval from original parent

## Testing Approach

### **Integration Test Strategy**
These tests use **real agents** and **real message passing** to validate actual system behavior, not mocked interactions. This approach ensures:

- Realistic timing and async behavior
- Actual inter-agent communication patterns  
- Real tool usage and coordination
- Proper state transitions across agent hierarchies
- Validation of the complete system workflow

### **Test Framework** (`mod.rs`)
The shared test framework provides:

1. **Agent Behavior Templates** - Pre-defined agent behaviors for different scenarios
2. **Workflow Orchestrator** - Helper to set up complex multi-agent scenarios  
3. **State Verification System** - Track agent states across entire hierarchy
4. **Timeline Assertions** - Verify message causality across multiple agents
5. **Mock Manager Responses** - Controllable manager behavior for approvals/rejections

### **Testing Phases**

**Phase 1: Core Mechanics** (Scenarios 1-6)  
Essential spawn/wait mechanics and basic plan approval workflows.

**Phase 2: Multi-Agent Coordination** (Scenarios 12-16)  
Multiple agent orchestration and tool usage coordination.

**Phase 3: Complex Workflows** (Scenarios 17-20)  
Nested spawning and multi-level agent hierarchies.

## Key Insights

These tests validate that the HIVE system correctly implements a **distributed state machine** where:
- Each agent can be in different states independently
- Overall system behavior emerges from agent interactions
- Message causality is preserved across agent boundaries
- Wait states properly coordinate parent-child relationships
- Tool usage is properly coordinated across multiple agents

The goal is confidence that the multi-agent orchestration system works reliably in all common scenarios that users will encounter.