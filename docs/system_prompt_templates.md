# System Prompt Templates

The copilot assistant now supports Jinja2 templates for system prompts, allowing you to dynamically customize the assistant's behavior based on available tools, system information, and other context.

## Basic Usage

You can use either a plain string or a Jinja template for the `system_prompt` in your configuration:

```toml
[model]
system_prompt = "You are a helpful assistant."  # Plain string
```

Or with a template:

```toml
[model]
system_prompt = """You are an assistant with access to {{ tools|length }} tools.
Current time: {{ current_datetime }}
System: {{ os }} ({{ arch }})"""
```

## Available Template Variables

The following variables are available in your system prompt templates:

- **`tools`**: List of available tools, each with:
  - `name`: The tool's name
  - `description`: The tool's description
- **`current_datetime`**: Current date and time in "YYYY-MM-DD HH:MM:SS" format
- **`os`**: Operating system (e.g., "macos", "linux", "windows")
- **`arch`**: System architecture (e.g., "x86_64", "aarch64")
- **`whitelisted_commands`**: List of commands that can be executed without user confirmation

## Example Templates

### List All Tools

```toml
system_prompt = """You are an AI assistant with the following tools available:

{% for tool in tools -%}
- {{ tool.name }}: {{ tool.description }}
{% endfor %}

Please use these tools appropriately to help the user."""
```

### Conditional Content

```toml
system_prompt = """You are an AI coding assistant.

{% if tools|length > 0 -%}
You have access to {{ tools|length }} tools to help with tasks.
{% else -%}
You have no tools available, so you can only provide advice.
{% endif %}

{% if "command" in tools|map(attribute="name") -%}
You can execute system commands using the command tool.
{% endif %}"""
```

### Include Whitelisted Commands

```toml
system_prompt = """You are a helpful assistant.

{% if whitelisted_commands -%}
The following commands can be executed without user confirmation:
{{ whitelisted_commands|join(', ') }}

All other commands will require user approval.
{% endif %}"""
```

## Template Syntax

The template engine supports standard Jinja2 syntax:

- **Variables**: `{{ variable_name }}`
- **Loops**: `{% for item in items %}...{% endfor %}`
- **Conditionals**: `{% if condition %}...{% endif %}`
- **Filters**: `{{ variable|filter_name }}`
- **Comments**: `{# This is a comment #}`

Common filters:
- `|length`: Get the length of a list
- `|join(', ')`: Join list items with a delimiter
- `|map(attribute="name")`: Extract an attribute from each item in a list

## Fallback Behavior

If template rendering fails (e.g., due to syntax errors), the system will:
1. Log an error message
2. Display the error in the TUI
3. Fall back to using the raw template string as the system prompt

## Testing Your Templates

To test if your template renders correctly, you can:

1. Check the logs for any template rendering errors
2. Observe the assistant's behavior to ensure it matches your expectations
3. Use the example configuration files in `examples/configs/` as a reference

## Migration from Static Prompts

If you're currently using a static system prompt, no changes are needed. The system automatically detects whether a prompt contains template syntax and processes it accordingly.

To convert a static prompt to a template:

1. Identify parts of your prompt that could benefit from dynamic content
2. Replace static text with template variables
3. Add conditionals or loops as needed

Example migration:

Before:
```toml
system_prompt = "You are a helpful assistant with access to various tools."
```

After:
```toml
system_prompt = """You are a helpful assistant with access to {{ tools|length }} tools.

Available tools: {{ tools|map(attribute="name")|join(', ') }}

Current time: {{ current_datetime }}"""
```