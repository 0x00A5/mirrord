## What is `mirrord`?

`mirrord` is a tool that allows developers to run local processes in the context of their k8s cloud 
environment. By skipping the process of CI, building image and deployment, it makes testing
much easier.

## How is it different from other solutions?

Unlike other solutions that connect users' local machine to their cluster, `mirrord` runs at
the local process level by intercepting its syscalls and proxying them to the cloud environment.
In the k8s cluster, `mirrord` runs an agent at the level of the target pod. running on the same
k8s node and executing the syscalls received from users' local process.

