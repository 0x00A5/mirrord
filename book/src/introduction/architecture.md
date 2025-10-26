## A 1000-foot view

Depending on how mirrord deploys and connects to its agent running in the cloud, the overall 
architecture and the components involved may vary. 

### Direct Kubernetes Port Forwarding

```mermaid
graph LR
    subgraph Local["💻 Local Machine"]
        direction TB
        subgraph Process["Local Process"]
            Layer[Layer]
        end
        IntProxy[Internal Proxy]
    end

    subgraph Cluster["☸️ Kubernetes Cluster"]
        direction TB
        subgraph TargetNS["Target Namespace"]
            Agent[Agent]
            Target[Target Pod]
        end
    end

    Layer<-->IntProxy
    IntProxy<-->Agent
```

### External Proxy

### `mirrord` Operator
