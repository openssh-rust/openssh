mod common;
use common::*;

use openssh::*;

#[tokio::test]
#[cfg_attr(not(ci), ignore)]
async fn it_connects() {
    for session in connects().await {
        session.check().await.unwrap();
        session.close().await.unwrap();
    }
}
