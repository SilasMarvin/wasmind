# TODO:
1. Create MDBook docs
2. Perform file by file code review and organization overview
3. Finalize tui display / tool status / fix graph status bar (make it part of system stats?) / why when something gets removed are we selecting randomly

Change InterruptAndForceStatus -> QueueStatusChange
- On the next submit, it prevents it and forces it into the new status
- This does not interrupt or pop tool calls of any kind
- Need to listen for QueueStatusChange instead of InterruptAndForceStatus
- Need to store it as an option status on self
- Need to check if we have a queueed status in the submit function before we set the status to Processing
- Need to update where InterruptAndForceStatus is used everywhere to QueueStatus

Change CompactConversation
- When compacting the conversation, let's search backwards until we find (assistant -> non-assistant message) and keep all messages backwards to that point
- In the assistant, where we `if let some on the Compact Converstion`, instead of completely replacing the old messages with the new ones, let's check:
    1. is the chat_history returned empty? If so completely replace the old convo with the empty new one
    2. If not, get the originating request id from the last assistant message in the new chat history and replace the old one up to the matching originating request id
        -- log an error if the new convo has an originating request id that is not in the old one
        -- if we can't find a last originating requst

Let's say after compacting we have the messages:
1. UserMessage (conversation compaction message)
2. Assistant (with og id a5z)
3. ToolCall

And the old messages look like:
1. System
2. System
3. UserMessage
4. Assistant (with og id a5z)
5. ToolCall
6. System

The new chat history after we merge them would look like:
1. UserMessage (conversation compaction message)
2. Assistant (with og id a5z)
3. ToolCall
4. System


When comapcting the convo, we should prune the end of the messges so it ends with an (assistant -> non-assistant message) as going backwards, anything after that will be a System message or a User message we probably don't want to compact and instead will end up in the chat anyways

