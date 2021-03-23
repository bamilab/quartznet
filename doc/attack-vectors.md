# Attack vectors

## Malicous peers

### Peer unwilling to accept children

An attack can be demonstrated in which a malicious party is able to saturate the network with many egos, where none of them are willing to accept any child peers.
This poses a problem because then new peers would be unable to join.
A mechanism is needed to prevent these peers from participating in the network.

To stop this attack, a new request message type can be implemented.
This request can be sent to the parent of this malicious node, which upon receiving this request, can choose to investigate the malicous node.
The parent node could create a random ego, and then attempt to connect to this accused node.
If no connection could be made, it would be safe to assume this child node is unwilling to accept child nodes in general.
So you can then drop this node.

It would still be possible though attack in this manner as it would still take up some extra bandwidth, but it wouldn't be too much.