class TransientFailure(Exception):
    """Raised by a handler when its own dependency (Neo4j, or anything else later) was
    unreachable - not the message's fault. The consumer requeues on this and backs off,
    instead of giving up and sending the message to the DLQ.
    """
