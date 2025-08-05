# TODO:
1. Add the starting scope to delegation_network_coordinator
2. Finalize file_interaction broadcasts
3. Finalize how shutdown should work for temporal actors? Should the assistant exit when done? I think so / then remove exit broadcasts in flag issue and report normal but before broadcasting tool call result have them broadcast an assistant update request to done so the actor shuts down
4. Create MDBook docs
5. Analyze ui system performance
6. Fix long actor names in ui that overflow the boxes
7. Maybe also show actor state in chat_history??
