# Helm Chart

Enable PVC for `data_dir` persistent storage:

```bash
helm upgrade --install --namespace krill krill charts/krill --set image.tag="latest" \
--set persistence.enabled=true  --set persistence.storageClass="basic-retain" --set persistence.accessMode="ReadWriteMany"
```

Create separate PVC for `rsync` and `rrdp` data directories;

```bash
helm upgrade --install --namespace krill krill charts/krill --set image.tag="latest" \
--set persistence.enabled=true --set persistence.rrdp.enabled=true --set persistence.rsync.enabled=true \
--set persistence.storageClass="basic-retain" --set persistence.rrdp.storageClass="basic-retain" --set persistence.rsync.storageClass="basic-retain" \
--set persistence.accessMode="ReadWriteMany" --set persistence.rrdp.accessMode="ReadWriteMany" --set persistence.rsync.accessMode="ReadWriteMany"
```

To run the Helm test of the Krill kubernetes deployment;

```bash
$ helm test krill
Pod krill-test-connection pending
Pod krill-test-connection pending
Pod krill-test-connection succeeded
NAME: krill
LAST DEPLOYED: Tue Jul 14 11:24:05 2020
NAMESPACE: krill
STATUS: deployed
REVISION: 1
TEST SUITE:     krill-test-connection
Last Started:   Tue Jul 14 11:25:17 2020
Last Completed: Tue Jul 14 11:25:24 2020
Phase:          Succeeded
NOTES:
1. Get the application URL by running these commands:
  export POD_NAME=$(kubectl get pods --namespace krill -l "app.kubernetes.io/name=krill,app.kubernetes.io/instance=krill" -o jsonpath="{.items[0].metadata.name}")
  echo "Visit http://127.0.0.1:3000 to use your application"
  kubectl --namespace krill port-forward $POD_NAME 3000:80
```
