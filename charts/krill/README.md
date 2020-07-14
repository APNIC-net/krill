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
