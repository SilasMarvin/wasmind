# Check Health Actor

*Example infrastructure actor for agent health monitoring within the Hive delegation network*

This infrastructure actor monitors AI agent conversations and periodically spawns health analyzer agents to assess agent performance, detect issues, and ensure agents are making appropriate progress on their tasks. It provides automated oversight and quality control for delegation networks. Unlike tool actors, this actor provides monitoring services rather than exposing tools to AI agents.

## Actor ID
`check_health`

## When You Might Want This Actor

Include this actor in your Hive configuration when you need:

- **Automated agent monitoring**: Periodic health checks on agent performance and progress
- **Issue detection**: Identify when agents are stuck, confused, or going off-track
- **Quality control**: Ensure agents are following best practices and making appropriate decisions
- **Performance assessment**: Evaluate if agents are effectively completing their assigned tasks
- **Early warning system**: Detect problems before they become critical failures
- **Conversation analysis**: Review agent transcripts for coherence and effectiveness

This actor is valuable for delegation networks that need automated supervision and quality assurance without constant human oversight.

## Messages Listened For

- `assistant::Request` - Captures conversation requests from the monitored agent
  - Stores the chat state and messages for analysis
  
- `assistant::Response` - Monitors agent responses to trigger health checks
  - Adds responses to transcript and initiates health analysis when appropriate

## Messages Broadcast

This actor primarily spawns analyzer agents rather than broadcasting messages directly. The spawned health analyzers handle reporting.

## Configuration

Requires configuration to set the health check interval:

```toml
[check_health]
check_interval = 300  # Check every 5 minutes (in seconds)
```

## How It Works

When activated in a Hive system, this actor:

1. **Monitors agent conversations** by capturing all requests and responses from the assigned scope
2. **Maintains conversation transcripts** for comprehensive health analysis
3. **Checks timing intervals** to determine when health checks should occur
4. **Spawns analyzer agents** that independently assess the monitored agent's health
5. **Provides automated oversight** without interfering with the agent's primary tasks
6. **Enables early intervention** by detecting issues before they impact task completion

The actor provides passive monitoring that spawns active health assessments, creating a non-intrusive supervision system for AI agents in delegation networks.